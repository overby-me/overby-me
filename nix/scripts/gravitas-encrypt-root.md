# Gravitas: Migrate root to LUKS2 + Nitrokey FIDO2 unlock

In-place shrink-and-shuffle migration of the `gravitas` ThinkPad's root
filesystem from plain btrfs to LUKS2-encrypted btrfs. Companion script:
`gravitas-encrypt-root.sh`.

The old filesystem on `/dev/nvme0n1p2` is **preserved untouched** until
you've successfully booted from the new encrypted filesystem and chosen
to reclaim its space. At every step before that, rolling back is just a
reboot away.

## Before booting the live USB

1. **Final backup check.** Confirm you've copied off-machine:
   - `/etc/age/fido2_host_key`
   - `/etc/ssh/ssh_host_ed25519_key{,.pub}`
   - `/etc/ssh/ssh_host_rsa_key{,.pub}`
   - `~/.ssh/`
   - `~/.gnupg/` (if you use GPG)
   - Anything else important in `~/` you'd cry over losing. The
      migration *should* preserve everything via `btrfs send`, but if
      something corrupts mid-flight you want to be able to reinstall.

2. **Confirm the feature branch is pushed.** The encryption changes
   live on the `encrypt-gravitas` bookmark on `origin`. Verify with
   `rtk jj log -r encrypt-gravitas` before powering off.

3. **Power off.**

   ```sh
   sudo poweroff
   ```

## Phase B — Boot the live USB

1. Insert USB, power on, hit F12 (ThinkPad boot menu), pick the USB.
2. At the live shell:

   ```sh
   sudo -i
   ping -c 2 1.1.1.1            # confirm networking
   ```

3. Clone the repo and check out the feature branch:

   ```sh
   cd /tmp
   # If git is not in PATH on the live image, wrap in `nix-shell -p git --run '...'`
   git clone https://tangled.org/overby.me/overby.me.git
   cd overby.me
   git checkout encrypt-gravitas
   ```

   Verify the tools the script needs are present:

   ```sh
   which cryptsetup btrfs sgdisk parted blkid pv
   ```

   On a recent NixOS minimal ISO they should all be available. If `pv`
   is missing, either install it (`nix-shell -p pv`) or edit the script
   to remove the `| pv -pterabT` from the `btrfs send` line.

## Phase C — Run the migration script (steps 1–7)

```sh
# Read it once, end to end, before running.
less nix/scripts/gravitas-encrypt-root.sh

# Then:
bash nix/scripts/gravitas-encrypt-root.sh
```

The script will:

1. Print the partition table + btrfs usage and ask you to type `YES` to
   confirm.
2. Shrink the btrfs filesystem to 930 G.
3. Shrink partition `/dev/nvme0n1p2` to 950 GiB.
4. Create new partition `/dev/nvme0n1p3` in the freed ~900 GiB tail.
5. `luksFormat` p3 — **prompts for a new passphrase**. Pick a strong
   one (this is the only thing standing between an attacker and your
   data if the Nitrokey is lost).
6. Open as `/dev/mapper/cryptroot`.
7. `mkfs.btrfs`, create subvolume `@`.
8. Snapshot the old `@` and `btrfs send | btrfs receive` it into the new
   filesystem. This is the long step — at ~1 TiB used and NVMe-internal
   copy, expect 30–90 minutes.
9. Print the new UUIDs you'll need for the next phase.

If anything fails before step 5 (`luksFormat`), the old filesystem is
intact and you can reboot back into it — nothing destructive has
happened. If anything fails *after* step 5 but before the new system
boots, the old filesystem is *still* intact (the script never touches
it).

## Phase D — Update the Nix config with real UUIDs

The script prints three UUIDs at the end. Edit the file (still on the
live USB):

```sh
$EDITOR /tmp/overby.me/nix/config/hardware/thinkpad-t14-ryzen-7-pro.nix
```

Replace:

| Placeholder | Replace with |
|-|-|
| `REPLACE-WITH-LUKS-PARTITION-UUID` | The LUKS UUID printed by the script |
| `REPLACE-WITH-INNER-BTRFS-UUID` | The inner btrfs UUID printed by the script |

The ESP UUID (currently `8AC7-C912`) should be unchanged — verify it
matches what the script printed.

## Phase E — Install the bootloader into the new filesystem

The script's epilogue gives you the exact commands. Adapted for the
clone path used above:

```sh
MAPPER=/dev/mapper/cryptroot
NEW_MNT=/mnt/dst
mount -o subvol=@,compress=zstd,noatime $MAPPER $NEW_MNT
mkdir -p $NEW_MNT/boot
mount /dev/nvme0n1p1 $NEW_MNT/boot

# Make the (edited) flake reachable from inside the chroot
mkdir -p $NEW_MNT/mnt/flake
mount --bind /tmp/overby.me $NEW_MNT/mnt/flake

# Build & install bootloader entry for the new generation
nixos-enter --root $NEW_MNT -- nixos-rebuild boot --flake /mnt/flake#gravitas
```

This will:

- Build the system with the new LUKS-aware config.
- Write a new systemd-boot entry into `/boot/loader/entries/`.
- Make the **next** boot use the encrypted filesystem.

## Phase F — First boot from the encrypted filesystem

```sh
umount $NEW_MNT/boot
umount $NEW_MNT
cryptsetup close cryptroot
reboot
```

Remove the USB. systemd-boot menu will appear with multiple entries —
the newest one is the encrypted one. Boot it.

You'll get a passphrase prompt from systemd-cryptsetup in initrd. Enter
the LUKS passphrase you set in Phase C step 5. System should boot
normally.

**If anything goes wrong at this stage**, just pick an older boot entry
from systemd-boot — those still point to the old plain-btrfs filesystem
on `/dev/nvme0n1p2`, which is untouched. You can reboot back into
normal life.

## Phase G — Enroll the Nitrokey

After the system is up on the encrypted filesystem, plug the Nitrokey in
and run:

```sh
sudo systemd-cryptenroll \
    --fido2-device=auto \
    --fido2-with-client-pin=yes \
    --fido2-with-user-presence=yes \
    /dev/nvme0n1p3
```

You'll be asked for:

1. The existing LUKS passphrase (to authorize adding a new keyslot).
2. The Nitrokey's FIDO2 PIN.
3. A touch on the Nitrokey button.

Reboot and verify FIDO2 unlock works. You should still be able to skip
the FIDO2 prompt and use the passphrase if you ever need to.

Verify the keyslots:

```sh
sudo systemd-cryptenroll /dev/nvme0n1p3
```

Should list:

```text
SLOT TYPE
   0 password
   1 fido2
```

### Optional: enroll a recovery key

```sh
sudo systemd-cryptenroll --recovery-key /dev/nvme0n1p3
```

It prints a long passphrase — **write it down on paper, lock it
somewhere physical**. It's your "I lost the Nitrokey *and* forgot the
passphrase" backup.

## Phase H — Reclaim the old partition (LATER)

The old filesystem on `/dev/nvme0n1p2` is wasting ~950 GiB and provides
no benefit once you trust the encrypted boot. Do this only after you've
used the encrypted system for a few days and are sure it's good. See
the **SECOND PASS** section at the bottom of `gravitas-encrypt-root.sh`
for options; the cleanest is from another live-USB session.

## Troubleshooting

| Problem | Recovery |
|-|-|
| Script fails before step 5 (LUKS format) | Reboot, you're back where you started |
| Script fails during `btrfs send/receive` | Old fs intact; `cryptsetup close cryptroot`, `sgdisk --delete=3 /dev/nvme0n1`, reboot to old fs, debug |
| `nixos-rebuild boot` fails inside `nixos-enter` | Old boot entries still in ESP; reboot, pick old entry |
| New system boots but something is broken | Pick old generation from systemd-boot |
| Forgot passphrase before enrolling Nitrokey | Game over — wipe and reinstall (this is why Phase H is deferred) |
| FIDO2 enrollment fails | Passphrase still works; debug at leisure |

## Heads-up before you start

- **The `btrfs send | receive` step takes time.**  For ~1 TiB of used
  data on internal NVMe doing simultaneous reads and writes, plan on
  30–90 minutes. Don't interrupt it.
- **Have your laptop on AC power** the entire time.
- **Re-read the script before running it.** An extra pair of eyes is
  cheap insurance.
- **The script stops before destroying the old filesystem.** That's
  intentional. Phase H is a *separate* later step.
