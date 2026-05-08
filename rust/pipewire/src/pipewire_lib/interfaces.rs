// PipeWire interface type names + opcode tables.
//
// These mirror the `PW_TYPE_INTERFACE_*` defines in
// `src/pipewire/{core,client,device,factory,link,module,node,port}.h` and
// `src/pipewire/extensions/*.h`. They are the strings the daemon emits in
// `Registry.Global` events.
//
// Also: per-interface method/event opcode tables matching
// `src/pipewire/{core,client,...}.h::PW_*_METHOD_*` / `PW_*_EVENT_*`.
//
// Versions match the upstream `PW_VERSION_*` constants for v1.6.

#![allow(dead_code)]

pub const TYPE_CORE: &str = "PipeWire:Interface:Core";
pub const TYPE_REGISTRY: &str = "PipeWire:Interface:Registry";
pub const TYPE_CLIENT: &str = "PipeWire:Interface:Client";
pub const TYPE_DEVICE: &str = "PipeWire:Interface:Device";
pub const TYPE_FACTORY: &str = "PipeWire:Interface:Factory";
pub const TYPE_LINK: &str = "PipeWire:Interface:Link";
pub const TYPE_MODULE: &str = "PipeWire:Interface:Module";
pub const TYPE_NODE: &str = "PipeWire:Interface:Node";
pub const TYPE_PORT: &str = "PipeWire:Interface:Port";
pub const TYPE_CLIENT_NODE: &str = "PipeWire:Interface:ClientNode";
pub const TYPE_METADATA: &str = "PipeWire:Interface:Metadata";
pub const TYPE_PROFILER: &str = "PipeWire:Interface:Profiler";
pub const TYPE_SECURITY_CONTEXT: &str = "PipeWire:Interface:SecurityContext";

pub const VERSION_CORE: u32 = 4;
pub const VERSION_REGISTRY: u32 = 3;
pub const VERSION_CLIENT: u32 = 3;
pub const VERSION_DEVICE: u32 = 3;
pub const VERSION_FACTORY: u32 = 3;
pub const VERSION_LINK: u32 = 3;
pub const VERSION_MODULE: u32 = 3;
pub const VERSION_NODE: u32 = 3;
pub const VERSION_PORT: u32 = 3;
pub const VERSION_METADATA: u32 = 3;
pub const VERSION_PROFILER: u32 = 1;

/// Special object ids that are pre-bound on every connection.
pub const ID_CORE: u32 = 0;
/// First proxy id used by the client when it allocates new ids. The client
/// proxy itself is at id=1 (ID_CLIENT).
pub const ID_CLIENT: u32 = 1;
/// Sentinel meaning "any proxy" (not a real id). Matches PW_ID_ANY.
pub const ID_ANY: u32 = 0xffff_ffff;

/// Core method opcodes (proxy → server).
pub mod core_method {
    pub const ADD_LISTENER: u8 = 0;
    pub const HELLO: u8 = 1;
    pub const SYNC: u8 = 2;
    pub const PONG: u8 = 3;
    pub const ERROR: u8 = 4;
    pub const GET_REGISTRY: u8 = 5;
    pub const CREATE_OBJECT: u8 = 6;
    pub const DESTROY: u8 = 7;
}

/// Core event opcodes (server → proxy).
pub mod core_event {
    pub const INFO: u8 = 0;
    pub const DONE: u8 = 1;
    pub const PING: u8 = 2;
    pub const ERROR: u8 = 3;
    pub const REMOVE_ID: u8 = 4;
    pub const BOUND_ID: u8 = 5;
    pub const ADD_MEM: u8 = 6;
    pub const REMOVE_MEM: u8 = 7;
    pub const BOUND_PROPS: u8 = 8;
}

/// Registry method opcodes.
pub mod registry_method {
    pub const ADD_LISTENER: u8 = 0;
    pub const BIND: u8 = 1;
    pub const DESTROY: u8 = 2;
}

/// Registry event opcodes.
pub mod registry_event {
    pub const GLOBAL: u8 = 0;
    pub const GLOBAL_REMOVE: u8 = 1;
}

/// Client method opcodes (only those we use).
pub mod client_method {
    pub const ADD_LISTENER: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const UPDATE_PROPERTIES: u8 = 2;
    pub const GET_PERMISSIONS: u8 = 3;
    pub const UPDATE_PERMISSIONS: u8 = 4;
}

/// Strip the `PipeWire:Interface:` prefix from a type string. Mirrors the
/// `spa_type_short_name` helper.
pub fn short_name(t: &str) -> &str {
    t.rsplit_once(':').map(|(_, s)| s).unwrap_or(t)
}

/// pw-cli's `pw_node_state_as_string`. State enum from `spa/node/node.h`:
/// 0=creating, 1=suspended, 2=idle, 3=running. -1 (0xFFFFFFFF on the wire
/// as an Id) is "error".
pub fn node_state_name(state: u32) -> &'static str {
    match state {
        0 => "creating",
        1 => "suspended",
        2 => "idle",
        3 => "running",
        _ => "error",
    }
}

/// Direction enum from `spa/utils/defs.h`: 0=input, 1=output.
pub fn direction_name(direction: u32) -> &'static str {
    match direction {
        0 => "input",
        1 => "output",
        _ => "unknown",
    }
}

/// Map a SPA_PARAM_* id to the `Spa:Enum:ParamId:Name` string used by
/// `spa_debug_type_find_name(spa_type_param, id)` in pw-cli's print_params.
/// Mirrors `spa/include/spa/param/param-types.h::spa_type_param`.
pub fn param_name(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "Spa:Enum:ParamId:Invalid",
        1 => "Spa:Enum:ParamId:PropInfo",
        2 => "Spa:Enum:ParamId:Props",
        3 => "Spa:Enum:ParamId:EnumFormat",
        4 => "Spa:Enum:ParamId:Format",
        5 => "Spa:Enum:ParamId:Buffers",
        6 => "Spa:Enum:ParamId:Meta",
        7 => "Spa:Enum:ParamId:IO",
        8 => "Spa:Enum:ParamId:EnumProfile",
        9 => "Spa:Enum:ParamId:Profile",
        10 => "Spa:Enum:ParamId:EnumPortConfig",
        11 => "Spa:Enum:ParamId:PortConfig",
        12 => "Spa:Enum:ParamId:EnumRoute",
        13 => "Spa:Enum:ParamId:Route",
        14 => "Spa:Enum:ParamId:Control",
        15 => "Spa:Enum:ParamId:Latency",
        16 => "Spa:Enum:ParamId:ProcessLatency",
        17 => "Spa:Enum:ParamId:Tag",
        18 => "Spa:Enum:ParamId:PeerEnumFormat",
        20 => "Spa:Enum:ParamId:Capability",
        21 => "Spa:Enum:ParamId:PeerCapability",
        _ => return None,
    })
}

/// Param info flags (from `spa/param/param.h`).
pub const PARAM_INFO_SERIAL: u32 = 1 << 0;
pub const PARAM_INFO_READ: u32 = 1 << 1;
pub const PARAM_INFO_WRITE: u32 = 1 << 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names() {
        assert_eq!(short_name(TYPE_CORE), "Core");
        assert_eq!(short_name(TYPE_REGISTRY), "Registry");
        assert_eq!(short_name(TYPE_CLIENT_NODE), "ClientNode");
        assert_eq!(short_name("plain"), "plain");
    }
}
