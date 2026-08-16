# Unit tests for MachineConfig and related contracts
#
# Validates that MachineConfig, UserConfig, FileSystemConfig, NetworkingConfig,
# FirewallConfig, StateVersion, and InterfaceConfig contracts correctly accept
# valid configurations, reject invalid ones, and apply defaults.
#
# Run with: nickel eval tests/unit/machine.ncl
let {
  MachineConfig,
  UserConfig,
  FileSystemConfig,
  NetworkingConfig,
  FirewallConfig,
  InterfaceConfig,
  StateVersion,
  ..
} = import "../../contracts/machine.ncl"
in
let { System, .. } = import "../../contracts/common.ncl" in

# ── Helpers ───────────────────────────────────────────────────────
let passes = fun contract value =>
  let wrapper =
    std.contract.custom (fun label v =>
      std.contract.check contract label v
      |> match {
        'Ok _ => 'Ok true,
        'Error _ => 'Ok false,
      }
    )
  in
  (value | wrapper)
in

let fails = fun contract value =>
  !(passes contract value)
in

let assert_ok = fun label value =>
  if value then
    "PASS: %{label}"
  else
    std.fail_with "FAIL: %{label}"
in

let assert_export = fun label expr =>
  let _ = std.serialize 'Json expr in
  "PASS: %{label} (exported successfully)"
in

# ── StateVersion ──────────────────────────────────────────────────
let state_version_tests = {
  valid_25_05 = passes StateVersion "25.05",
  valid_24_11 = passes StateVersion "24.11",
  valid_23_05 = passes StateVersion "23.05",
  valid_00_00 = passes StateVersion "00.00",
  valid_99_99 = passes StateVersion "99.99",

  rejects_empty = fails StateVersion "",
  rejects_no_dot = fails StateVersion "2505",
  rejects_single_digit = fails StateVersion "5.05",
  rejects_three_digit = fails StateVersion "025.05",
  rejects_text = fails StateVersion "unstable",
  rejects_number = fails StateVersion 25,
  rejects_bool = fails StateVersion true,
  rejects_null = fails StateVersion null,
  rejects_extra_dot = fails StateVersion "25.05.1",
  rejects_leading_v = fails StateVersion "v25.05",
}
in

# ── Minimal MachineConfig ────────────────────────────────────────
let test_minimal =
  let config | MachineConfig = {
    system = "x86_64-linux",
  }
  in
  [
    assert_ok "minimal machine has system" (config.system == "x86_64-linux"),
    assert_ok "minimal machine default state-version" (config.state-version == "25.05"),
    assert_ok "minimal machine default modules" (config.modules == []),
    assert_ok "minimal machine default special-args" (config.special-args == {}),
    assert_ok "minimal machine default users" (config.users == {}),
    assert_ok "minimal machine default boot-loader" (config.boot-loader == 'systemd-boot),
    assert_ok "minimal machine default file-systems" (config.file-systems == {}),
    assert_ok "minimal machine default networking use-dhcp" (config.networking.use-dhcp == true),
    assert_ok "minimal machine default extra-config" (config.extra-config == {}),
  ]
in

# ── Full MachineConfig ───────────────────────────────────────────
let test_full =
  let config | MachineConfig = {
    system = "aarch64-linux",
    state-version = "24.11",
    modules = ["desktop", "development"],
    host-name = "gravitas",
    special-args = {
      myFlag = true,
    },
    users = {
      alice = {
        home-manager = true,
        home-modules = ["shell", "editor"],
        extra-groups = ["wheel", "video"],
        shell = "zsh",
        is-normal-user = true,
      },
    },
    boot-loader = 'grub,
    file-systems = {
      "/" = {
        device = "/dev/disk/by-label/nixos",
        fs-type = "ext4",
      },
      "/boot" = {
        device = "/dev/disk/by-label/boot",
        fs-type = "vfat",
        needed-for-boot = true,
      },
    },
    networking = {
      use-dhcp = false,
      firewall = {
        enable = true,
        allowed-tcp-ports = [22, 80, 443],
        allowed-udp-ports = [53],
      },
      interfaces = {
        eth0 = {
          use-dhcp = true,
        },
      },
      wireless = false,
    },
    time-zone = "Europe/Copenhagen",
    locale = "en_US.UTF-8",
    extra-config = {
      services.openssh.enable = true,
    },
  }
  in
  [
    assert_ok "full machine system" (config.system == "aarch64-linux"),
    assert_ok "full machine state-version" (config.state-version == "24.11"),
    assert_ok "full machine modules count" (std.array.length config.modules == 2),
    assert_ok "full machine host-name" (config.host-name == "gravitas"),
    assert_ok "full machine boot-loader" (config.boot-loader == 'grub),
    assert_ok "full machine time-zone" (config.time-zone == "Europe/Copenhagen"),
    assert_ok "full machine locale" (config.locale == "en_US.UTF-8"),
    assert_ok "full machine has users" (std.record.has_field "alice" config.users),
    assert_ok "full machine alice groups" (std.array.length config.users.alice.extra-groups == 2),
    assert_ok "full machine alice shell" (config.users.alice.shell == "zsh"),
    assert_ok "full machine has root fs" (std.record.has_field "/" config.file-systems),
    assert_ok "full machine root device" (config.file-systems."/".device == "/dev/disk/by-label/nixos"),
    assert_ok "full machine boot needed" (config.file-systems."/boot".needed-for-boot == true),
    assert_ok "full machine firewall enabled" (config.networking.firewall.enable == true),
    assert_ok "full machine tcp ports" (std.array.length config.networking.firewall.allowed-tcp-ports == 3),
    assert_ok "full machine wireless" (config.networking.wireless == false),
  ]
in

# ── Boot loader variants ─────────────────────────────────────────
let test_boot_loader_systemd_boot =
  let config | MachineConfig = {
    system = "x86_64-linux",
    boot-loader = 'systemd-boot,
  }
  in
  assert_ok "boot-loader systemd-boot" (config.boot-loader == 'systemd-boot)
in

let test_boot_loader_grub =
  let config | MachineConfig = {
    system = "x86_64-linux",
    boot-loader = 'grub,
  }
  in
  assert_ok "boot-loader grub" (config.boot-loader == 'grub)
in

let test_boot_loader_none =
  let config | MachineConfig = {
    system = "x86_64-linux",
    boot-loader = 'none,
  }
  in
  assert_ok "boot-loader none" (config.boot-loader == 'none)
in

# ── System variants ──────────────────────────────────────────────
let test_all_systems =
  let mk = fun sys =>
    let config | MachineConfig = { system = sys } in
    config.system == sys
  in
  [
    assert_ok "machine x86_64-linux" (mk "x86_64-linux"),
    assert_ok "machine aarch64-linux" (mk "aarch64-linux"),
    assert_ok "machine x86_64-darwin" (mk "x86_64-darwin"),
    assert_ok "machine aarch64-darwin" (mk "aarch64-darwin"),
  ]
in

# ── UserConfig ────────────────────────────────────────────────────
let test_user_defaults =
  let user | UserConfig = {} in
  [
    assert_ok "user default home-manager" (user.home-manager == true),
    assert_ok "user default home-modules" (user.home-modules == []),
    assert_ok "user default extra-groups" (user.extra-groups == []),
    assert_ok "user default is-normal-user" (user.is-normal-user == true),
  ]
in

let test_user_full =
  let user | UserConfig = {
    home-manager = true,
    home-modules = ["shell", "editor", "git"],
    extra-groups = ["wheel", "docker", "video"],
    shell = "fish",
    is-normal-user = true,
  }
  in
  [
    assert_ok "user full home-manager" (user.home-manager == true),
    assert_ok "user full home-modules count" (std.array.length user.home-modules == 3),
    assert_ok "user full groups count" (std.array.length user.extra-groups == 3),
    assert_ok "user full shell" (user.shell == "fish"),
    assert_ok "user full is-normal-user" (user.is-normal-user == true),
  ]
in

let test_system_user =
  let user | UserConfig = {
    is-normal-user = false,
    home-manager = false,
  }
  in
  [
    assert_ok "system user is-normal-user" (user.is-normal-user == false),
    assert_ok "system user no home-manager" (user.home-manager == false),
  ]
in

# ── FileSystemConfig ─────────────────────────────────────────────
let test_fs_minimal =
  let fs | FileSystemConfig = {
    device = "/dev/sda1",
  }
  in
  [
    assert_ok "fs minimal device" (fs.device == "/dev/sda1"),
    assert_ok "fs minimal default type" (fs.fs-type == "ext4"),
    assert_ok "fs minimal default options" (fs.options == []),
    assert_ok "fs minimal default needed-for-boot" (fs.needed-for-boot == false),
  ]
in

let test_fs_full =
  let fs | FileSystemConfig = {
    device = "UUID=abcd-1234",
    fs-type = "btrfs",
    options = ["compress=zstd", "noatime"],
    needed-for-boot = true,
  }
  in
  [
    assert_ok "fs full device" (fs.device == "UUID=abcd-1234"),
    assert_ok "fs full type" (fs.fs-type == "btrfs"),
    assert_ok "fs full options count" (std.array.length fs.options == 2),
    assert_ok "fs full needed-for-boot" (fs.needed-for-boot == true),
  ]
in

# ── NetworkingConfig ──────────────────────────────────────────────
let test_networking_defaults =
  let net | NetworkingConfig = {} in
  [
    assert_ok "networking default use-dhcp" (net.use-dhcp == true),
    assert_ok "networking default firewall enable" (net.firewall.enable == true),
    assert_ok "networking default firewall tcp" (net.firewall.allowed-tcp-ports == []),
    assert_ok "networking default firewall udp" (net.firewall.allowed-udp-ports == []),
    assert_ok "networking default interfaces" (net.interfaces == {}),
    assert_ok "networking default wireless" (net.wireless == false),
  ]
in

let test_networking_full =
  let net | NetworkingConfig = {
    use-dhcp = false,
    firewall = {
      enable = true,
      allowed-tcp-ports = [22, 80, 443, 8080],
      allowed-udp-ports = [53, 5353],
    },
    interfaces = {
      eth0 = {
        use-dhcp = true,
      },
      wlan0 = {
        use-dhcp = false,
        ip-addresses = ["192.168.1.100/24"],
      },
    },
    wireless = true,
  }
  in
  [
    assert_ok "networking full use-dhcp" (net.use-dhcp == false),
    assert_ok "networking full firewall tcp count" (std.array.length net.firewall.allowed-tcp-ports == 4),
    assert_ok "networking full firewall udp count" (std.array.length net.firewall.allowed-udp-ports == 2),
    assert_ok "networking full has eth0" (std.record.has_field "eth0" net.interfaces),
    assert_ok "networking full has wlan0" (std.record.has_field "wlan0" net.interfaces),
    assert_ok "networking full wireless" (net.wireless == true),
  ]
in

# ── FirewallConfig ────────────────────────────────────────────────
let test_firewall_defaults =
  let fw | FirewallConfig = {} in
  [
    assert_ok "firewall default enable" (fw.enable == true),
    assert_ok "firewall default tcp" (fw.allowed-tcp-ports == []),
    assert_ok "firewall default udp" (fw.allowed-udp-ports == []),
  ]
in

let test_firewall_disabled =
  let fw | FirewallConfig = {
    enable = false,
  }
  in
  assert_ok "firewall disabled" (fw.enable == false)
in

# ── InterfaceConfig ───────────────────────────────────────────────
let test_interface_defaults =
  let iface | InterfaceConfig = {} in
  [
    assert_ok "interface default use-dhcp" (iface.use-dhcp == true),
    assert_ok "interface default ip-addresses" (iface.ip-addresses == []),
  ]
in

let test_interface_static =
  let iface | InterfaceConfig = {
    use-dhcp = false,
    ip-addresses = ["10.0.0.1/24", "10.0.0.2/24"],
  }
  in
  [
    assert_ok "interface static use-dhcp" (iface.use-dhcp == false),
    assert_ok "interface static ip count" (std.array.length iface.ip-addresses == 2),
  ]
in

# ── Invalid validator tests (passes/fails works for validator contracts) ──
let invalid_validator_tests = {
  # StateVersion validator rejects bad formats
  rejects_state_version_text = fails StateVersion "unstable",
  rejects_state_version_number = fails StateVersion 25,
  rejects_state_version_empty = fails StateVersion "",

  # System validator rejects bad values (used by MachineConfig.system)
  rejects_system_powerpc = fails System "powerpc-linux",
  rejects_system_typo = fails System "x86-linux",
}
in

# ── Invalid record configs (tested via export forcing evaluation) ──
# Record contracts like MachineConfig are lazy — we must force full
# evaluation with std.serialize to trigger missing-field errors.
# These are tested via error snapshot tests in tests/errors/.

# ── Export round-trip ─────────────────────────────────────────────
let test_export_minimal =
  let config | MachineConfig = {
    system = "x86_64-linux",
  }
  in
  assert_export "minimal machine exports to JSON" config
in

let test_export_full =
  let config | MachineConfig = {
    system = "aarch64-linux",
    state-version = "24.11",
    modules = ["desktop"],
    host-name = "test-host",
    file-systems = {
      "/" = {
        device = "/dev/sda1",
        fs-type = "ext4",
      },
    },
    networking = {
      firewall = {
        allowed-tcp-ports = [22],
      },
    },
    time-zone = "UTC",
    locale = "C.UTF-8",
    users = {
      testuser = {
        extra-groups = ["wheel"],
      },
    },
  }
  in
  assert_export "full machine exports to JSON" config
in

# ── Collect all results ──────────────────────────────────────────
let all_validator_tests =
  std.record.to_array state_version_tests
  @ std.record.to_array invalid_validator_tests
in

let validator_failures =
  all_validator_tests
  |> std.array.filter (fun entry => entry.value == false)
  |> std.array.map (fun entry => entry.field)
in

let all_assert_results =
  test_minimal
  @ test_full
  @ test_all_systems
  @ test_user_defaults
  @ test_user_full
  @ test_system_user
  @ test_fs_minimal
  @ test_fs_full
  @ test_networking_defaults
  @ test_networking_full
  @ test_firewall_defaults
  @ test_interface_defaults
  @ test_interface_static
  @ [
    test_boot_loader_systemd_boot,
    test_boot_loader_grub,
    test_boot_loader_none,
    test_firewall_disabled,
    test_export_minimal,
    test_export_full,
  ]
in

let total_count =
  std.array.length all_validator_tests
  + std.array.length all_assert_results
in

if std.array.length validator_failures > 0 then
  std.fail_with "FAILED validator tests: %{std.string.join ", " validator_failures}"
else
  let _ = all_assert_results |> std.array.map (fun r => std.trace r null) in
  "All %{std.to_string total_count} MachineConfig tests passed."
