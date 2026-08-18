# Machine contracts for nix-workspace
#
# Defines the MachineConfig contract for NixOS machine configurations
# discovered from the machines/ convention directory or declared in workspace.ncl.
#
# Each machine config maps to a nixosConfigurations.<name> flake output
# and is built using nixpkgs.lib.nixosSystem.
let { System, ModuleRef, NonEmptyString, .. } = import "./common.ncl" in

{
  StateVersion
    | doc m%"
      A NixOS state version string (e.g. "24.11", "25.05").
      Must match the pattern <major>.<minor> where both are two-digit numbers.
    "%
    =
      std.contract.from_validator (fun value =>
        if !(std.is_string value) then
          'Error {
            message = "expected a string, got %{std.typeof value}",
            notes = [
              "State version must be a string like \"25.05\".",
            ],
          }
        else if std.string.is_match "^[0-9]{2}\\.[0-9]{2}$" value then
          'Ok
        else
          'Error {
            message = "invalid state version \"%{value}\"",
            notes = [
              "State version must match the pattern \"YY.MM\" (e.g. \"24.11\", \"25.05\").",
              "This corresponds to the NixOS release version.",
            ],
          }
      ),

  MachineConfig = {
    system
      | System
      | doc "Target system architecture for this machine",

    state-version
      | StateVersion
      | doc m%"
          NixOS state version (e.g. "25.05").
          This determines the default versions of stateful data formats
          and should match the NixOS release you initially installed.
        "%
      | default
      = "25.05",

    modules
      | Array ModuleRef
      | doc m%"
          NixOS modules to include in this machine's configuration.
          Can be module names (resolved from the workspace's modules/ directory)
          or paths to .nix files.
        "%
      | default
      = [],

    host-name
      | String
      | doc m%"
          The machine's hostname. If not set, defaults to the machine name
          (the key in the machines record or the .ncl filename).
        "%
      | optional,

    special-args
      | { _ : Dyn }
      | doc m%"
          Extra arguments passed to the NixOS module system via specialArgs.
          These are available in all modules as top-level function arguments.
        "%
      | default
      = {},

    users
      | { _ : UserConfig }
      | doc m%"
          Per-user home-manager configurations for this machine.
          Keys are usernames; values are home-manager config references.
        "%
      | default
      = {},

    boot-loader
      | [| 'systemd-boot, 'grub, 'none |]
      | doc m%"
          Boot loader to configure. Defaults to systemd-boot.
          Set to 'none to manage boot loader configuration manually in modules.
        "%
      | default
      = 'systemd-boot,

    file-systems
      | { _ : FileSystemConfig }
      | doc m%"
          File system mount points. Keys are mount paths (e.g. "/", "/home").
          Basic file systems can be declared here; complex setups should use modules.
        "%
      | default
      = {},

    networking
      | NetworkingConfig
      | doc "Basic networking configuration for this machine"
      | default
      = {},

    time-zone
      | String
      | doc "Time zone for this machine (e.g. \"Europe/Copenhagen\")"
      | optional,

    locale
      | String
      | doc "Default locale (e.g. \"en_US.UTF-8\")"
      | optional,

    extra-config
      | { _ : Dyn }
      | doc m%"
          Escape hatch: arbitrary NixOS configuration options passed through
          verbatim to the NixOS module system. Use sparingly — prefer
          writing dedicated modules instead.
        "%
      | default
      = {},
  },

  UserConfig = {
    home-manager
      | Bool
      | doc "Whether to enable home-manager for this user"
      | default
      = true,

    home-modules
      | Array ModuleRef
      | doc m%"
          Home-manager modules to include for this user.
          Resolved from the workspace's home/ directory or as paths.
        "%
      | default
      = [],

    extra-groups
      | Array String
      | doc "Additional groups for this user"
      | default
      = [],

    shell
      | String
      | doc "Login shell package name (e.g. \"zsh\", \"fish\")"
      | optional,

    is-normal-user
      | Bool
      | doc "Whether this is a normal (non-system) user account"
      | default
      = true,
  },

  FileSystemConfig = {
    device
      | NonEmptyString
      | doc "Device path, UUID, or label (e.g. \"/dev/sda1\", \"UUID=...\")",

    fs-type
      | String
      | doc "File system type (e.g. \"ext4\", \"btrfs\", \"vfat\")"
      | default
      = "ext4",

    options
      | Array String
      | doc "Mount options"
      | default
      = [],

    needed-for-boot
      | Bool
      | doc "Whether this file system is needed during early boot"
      | default
      = false,
  },

  NetworkingConfig = {
    use-dhcp
      | Bool
      | doc "Enable DHCP globally (usually you want per-interface DHCP instead)"
      | default
      = true,

    firewall
      | FirewallConfig
      | doc "Firewall configuration"
      | default
      = {},

    interfaces
      | { _ : InterfaceConfig }
      | doc "Network interface configurations"
      | default
      = {},

    wireless
      | Bool
      | doc "Enable wireless networking support"
      | default
      = false,
  },

  FirewallConfig = {
    enable
      | Bool
      | doc "Enable the NixOS firewall"
      | default
      = true,

    allowed-tcp-ports
      | Array std.number.Integer
      | doc "TCP ports to allow through the firewall"
      | default
      = [],

    allowed-udp-ports
      | Array std.number.Integer
      | doc "UDP ports to allow through the firewall"
      | default
      = [],
  },

  InterfaceConfig = {
    use-dhcp
      | Bool
      | doc "Enable DHCP on this interface"
      | default
      = true,

    ip-addresses
      | Array String
      | doc "Static IP addresses in CIDR notation (e.g. \"192.168.1.100/24\")"
      | default
      = [],
  },
}
