#!/usr/bin/env bash
#
# In-place shrink-and-shuffle migration of gravitas root filesystem
# from plain btrfs to LUKS2-encrypted btrfs.
#
# RUN FROM A NIXOS LIVE USB.  Do NOT run this from the installed system.
#
# Strategy:
#   1. Shrink the existing btrfs on /dev/nvme0n1p2 to ~900 GiB.
#   2. Shrink the partition itself to match.
#   3. Create a new partition /dev/nvme0n1p3 in the freed tail.
#   4. LUKS2-format and open the new partition as 'cryptroot'.
#   5. mkfs.btrfs on the new mapped device, create subvolume @.
#   6. btrfs send | btrfs receive a read-only snapshot of the old @
#      into the new fs (preserves compression + reflinks, atomic).
#   7. Unmount and close everything; verify the new fs is bootable.
#   8. Manual second pass (after first successful boot from the new fs):
#      delete the old partition, grow p3 + LUKS + btrfs to fill the disk.
#
# This script INTENTIONALLY stops after step 7 so you can boot once
# from the encrypted fs (with the old plain fs still intact as a
# fallback) before destroying the source.  After confirming the
# encrypted boot works, run the second-pass commands at the bottom
# of this file.
#
# Pre-flight assumptions (verify before running!):
#   - Disk is /dev/nvme0n1
#   - p1 = ESP at /dev/nvme0n1p1 (vfat, ~1 GiB), unchanged
#   - p2 = current btrfs root at /dev/nvme0n1p2 (~1.8 TiB, 59% used = ~1.0 TiB)
#   - No swap partition, no other partitions
#   - You have a NixOS live USB with: cryptsetup, btrfs-progs, parted,
#     gdisk, util-linux.  The standard NixOS minimal ISO has all of these.
#   - You have backed up /etc/age/fido2_host_key, /etc/ssh/ssh_host_*,
#     ~/.ssh, and ~/.gnupg to external media.

set -euo pipefail

DISK=/dev/nvme0n1
OLD_PART=${DISK}p2
NEW_PART=${DISK}p3
OLD_MNT=/mnt/src
NEW_MNT=/mnt/dst
MAPPER_NAME=cryptroot
MAPPER=/dev/mapper/${MAPPER_NAME}

# Target size (in MiB) for the shrunk old partition.  Must comfortably
# exceed actual used data (~1.0 TiB) plus btrfs overhead/slack.
SHRUNK_SIZE_MIB=$((950 * 1024))   # 950 GiB

# Btrfs internal resize target.  Should be slightly smaller than the
# partition shrink to avoid edge cases.
BTRFS_RESIZE_TARGET="930G"

msg() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
warn() { printf '\n\033[1;33m!!  %s\033[0m\n' "$*" >&2; }
die() { printf '\n\033[1;31mXX  %s\033[0m\n' "$*" >&2; exit 1; }

confirm() {
  local prompt="$1"
  read -rp "$prompt [type YES to continue] " ans
  [[ "$ans" == "YES" ]] || die "Aborted by user."
}

require_root() {
  [[ $EUID -eq 0 ]] || die "Must run as root."
}

require_unmounted() {
  if findmnt -n "$OLD_PART" >/dev/null 2>&1; then
    die "$OLD_PART is mounted.  Boot from a live USB and do NOT mount it from the installer."
  fi
}

require_root
require_unmounted

msg "Pre-flight: showing current partition table and btrfs state"
parted "$DISK" -s unit GiB print
echo
msg "Mounting old fs read-only to inspect"
mkdir -p "$OLD_MNT"
mount -o ro "$OLD_PART" "$OLD_MNT"
btrfs filesystem show "$OLD_MNT"
btrfs filesystem usage "$OLD_MNT" | head -20
echo
df -h "$OLD_MNT"
umount "$OLD_MNT"

confirm "Above looks correct?  Will shrink ${OLD_PART} to ${SHRUNK_SIZE_MIB} MiB."

# -------------------------------------------------------------------------
# Step 1: btrfs internal resize (must be done while mounted RW)
# -------------------------------------------------------------------------
msg "Step 1: Shrinking btrfs internally to ${BTRFS_RESIZE_TARGET}"
mount "$OLD_PART" "$OLD_MNT"
btrfs filesystem resize "$BTRFS_RESIZE_TARGET" "$OLD_MNT"
btrfs filesystem show "$OLD_MNT"
umount "$OLD_MNT"

# -------------------------------------------------------------------------
# Step 2: shrink the partition itself
# -------------------------------------------------------------------------
msg "Step 2: Shrinking partition ${OLD_PART}"
# parted resizepart uses end position.  We compute end = start + size.
# Easier: use sgdisk to delete + recreate p2 with same start, new size.
# Capture original start sector first.
START_SECTOR=$(sgdisk -i 2 "$DISK" | awk '/First sector/ {print $3}')
[[ -n "$START_SECTOR" ]] || die "Could not determine p2 start sector"
# 2048 sectors/MiB at 512-byte sectors; nvme is usually 512.  Verify.
SECTOR_SIZE=$(blockdev --getss "$DISK")
[[ "$SECTOR_SIZE" -eq 512 ]] || die "Unexpected sector size $SECTOR_SIZE; adjust script."
END_SECTOR=$((START_SECTOR + SHRUNK_SIZE_MIB * 2048 - 1))
msg "p2: start=${START_SECTOR}  new_end=${END_SECTOR}  (size=${SHRUNK_SIZE_MIB} MiB)"

sgdisk --delete=2 "$DISK"
sgdisk --new=2:"${START_SECTOR}":"${END_SECTOR}" \
       --typecode=2:8300 \
       --change-name=2:"nixos-old" \
       "$DISK"
partprobe "$DISK"
sleep 2

# -------------------------------------------------------------------------
# Step 3: create new partition in the tail
# -------------------------------------------------------------------------
msg "Step 3: Creating new partition ${NEW_PART} in freed space"
sgdisk --new=3:0:0 \
       --typecode=3:8300 \
       --change-name=3:"nixos-cryptroot" \
       "$DISK"
partprobe "$DISK"
sleep 2
parted "$DISK" -s unit GiB print

# -------------------------------------------------------------------------
# Step 4: LUKS2 format + open
# -------------------------------------------------------------------------
msg "Step 4: LUKS2 formatting ${NEW_PART}"
warn "You will be asked for a NEW passphrase.  Pick a strong one — this is the only thing standing between an attacker and your data."
cryptsetup luksFormat \
  --type luks2 \
  --cipher aes-xts-plain64 \
  --key-size 512 \
  --hash sha256 \
  --pbkdf argon2id \
  --use-random \
  "$NEW_PART"

msg "Opening LUKS container as ${MAPPER_NAME}"
cryptsetup open "$NEW_PART" "$MAPPER_NAME"

# -------------------------------------------------------------------------
# Step 5: mkfs + subvolume layout
# -------------------------------------------------------------------------
msg "Step 5: Creating btrfs on ${MAPPER}"
mkfs.btrfs -L nixos "$MAPPER"

mkdir -p "$NEW_MNT"
mount -o compress=zstd,noatime "$MAPPER" "$NEW_MNT"
btrfs subvolume create "$NEW_MNT/@"
umount "$NEW_MNT"

# -------------------------------------------------------------------------
# Step 6: btrfs send | receive
# -------------------------------------------------------------------------
msg "Step 6: Snapshotting + sending old @ to new fs"
mount -o subvol=@ "$OLD_PART" "$OLD_MNT"
# btrfs send requires a read-only snapshot.
mkdir -p "$OLD_MNT/.snapshots"
btrfs subvolume snapshot -r "$OLD_MNT" "$OLD_MNT/.snapshots/migrate-src" || {
  # If that fails because '@' isn't a subvolume root from this mount, fall back
  # to mounting the top-level and snapshotting from there.
  umount "$OLD_MNT"
  mount -o subvolid=5 "$OLD_PART" "$OLD_MNT"
  btrfs subvolume snapshot -r "$OLD_MNT/@" "$OLD_MNT/migrate-src"
}
sync

# Mount destination top-level (subvolid=5) so we receive into it,
# then the received subvolume becomes a sibling of @.
mount "$MAPPER" "$NEW_MNT"

# Find the snapshot path we created.
if [[ -d "$OLD_MNT/.snapshots/migrate-src" ]]; then
  SNAP_PATH="$OLD_MNT/.snapshots/migrate-src"
else
  SNAP_PATH="$OLD_MNT/migrate-src"
fi

msg "Sending ${SNAP_PATH} -> ${NEW_MNT} (this will take a while)"
btrfs send "$SNAP_PATH" | pv -pterabT | btrfs receive "$NEW_MNT"

# The received subvolume will be named 'migrate-src'.  We need it named '@'.
# First delete the empty @ we created in step 5, then rename.
btrfs subvolume delete "$NEW_MNT/@"
mv "$NEW_MNT/migrate-src" "$NEW_MNT/@"
# Make it writable (received subvolumes are RO by default).
btrfs property set -ts "$NEW_MNT/@" ro false

umount "$NEW_MNT"
umount "$OLD_MNT"

# -------------------------------------------------------------------------
# Step 7: capture UUIDs and prep for nixos-enter / nixos-rebuild
# -------------------------------------------------------------------------
msg "Step 7: Capturing UUIDs for the NixOS config"
LUKS_UUID=$(blkid -s UUID -o value "$NEW_PART")
BTRFS_UUID=$(blkid -s UUID -o value "$MAPPER")
ESP_UUID=$(blkid -s UUID -o value "${DISK}p1")

cat <<EOF

============================================================
MIGRATION CORE COMPLETE.  Update the NixOS config with:

  LUKS device UUID (set on boot.initrd.luks.devices.cryptroot.device):
    /dev/disk/by-uuid/${LUKS_UUID}

  Inner btrfs UUID (set on fileSystems."/".device):
    /dev/disk/by-uuid/${BTRFS_UUID}

  ESP UUID (unchanged, sanity-check fileSystems."/boot".device):
    /dev/disk/by-uuid/${ESP_UUID}

Now, to install the bootloader and kernel into the new fs:

  mount -o subvol=@,compress=zstd,noatime ${MAPPER} ${NEW_MNT}
  mkdir -p ${NEW_MNT}/boot
  mount ${DISK}p1 ${NEW_MNT}/boot
  # Bind-mount nix store + the flake repo so nixos-enter can rebuild:
  mkdir -p ${NEW_MNT}/mnt/flake
  mount --bind /path/to/your/cloned/repo ${NEW_MNT}/mnt/flake
  nixos-enter --root ${NEW_MNT} -- nixos-rebuild boot --flake /mnt/flake#gravitas

  # Then while still inside nixos-enter:
  systemd-cryptenroll --fido2-device=auto ${NEW_PART}
  # (touch the Nitrokey when prompted)

Reboot.  At the boot menu pick the new generation.  systemd-boot will
hand off to systemd-initrd which will prompt for the LUKS passphrase
(or auto-unlock via the Nitrokey if FIDO2 enrollment succeeded).

============================================================

SECOND PASS (only after a successful boot from the encrypted fs):

  # From the live USB again, with old fs unmounted:
  sgdisk --delete=2 ${DISK}
  partprobe ${DISK}
  # Move p3 to start where p2 used to start (sgdisk can't move; use
  # sfdisk or recreate manually).  Easier: leave the layout alone
  # and just grow the LUKS+btrfs into the freed tail.  But the freed
  # space is BEFORE p3, not after, so a true reclaim requires a move.
  #
  # Recommended: skip the move; create p4 in the freed head, add it
  # to the btrfs as a second device (btrfs device add /dev/mapper/...
  # after wrapping it in its own LUKS).  Or simply leave ~950 GiB
  # unallocated for future use.  Discuss before doing this step.

EOF
