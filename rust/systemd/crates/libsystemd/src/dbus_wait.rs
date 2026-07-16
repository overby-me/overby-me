//! Systemd has the feature to wait on services that have type dbus. This means it waits until a specific name has been grabbed on the bus.
//!
//! This is made optional here to not have a hard dependency on dbus.

#[cfg(feature = "dbus_support")]
pub use dbus_support::*;

#[cfg(not(feature = "dbus_support"))]
pub use no_dbus_support::*;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum WaitResult {
    Ok,
    Timedout,
}

#[cfg(not(feature = "dbus_support"))]
mod no_dbus_support {

    use super::WaitResult;
    use std::error::Error;

    pub fn wait_for_name_system_bus(
        _name: &str,
        _timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult, Box<dyn Error>> {
        Err("Dbus is not supported in this build")?;

        // remove warnings about unused code in the enum
        let _ = WaitResult::Ok;
        let _ = WaitResult::Timedout;
        unreachable!();
    }

    // just used for testing
    #[allow(dead_code)]
    pub fn wait_for_name_session_bus(
        _name: &str,
        _timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult, Box<dyn Error>> {
        Err("Dbus is not supported in this build")?;
        unreachable!();
    }
}

#[cfg(feature = "dbus_support")]
mod dbus_support {

    use super::WaitResult;
    use std::sync::{Arc, Mutex};
    use zbus::blocking::Connection;
    use zbus::blocking::fdo::DBusProxy;
    use zbus::names::{BusName, OwnedBusName, WellKnownName};

    pub fn wait_for_name_system_bus(
        name: &str,
        timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult, Box<dyn std::error::Error>> {
        let conn = Connection::system()?;
        wait_for_name(name, &conn, timeout)
    }

    // just used for testing
    #[allow(dead_code)]
    pub fn wait_for_name_session_bus(
        name: &str,
        timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult, Box<dyn std::error::Error>> {
        let conn = Connection::session()?;
        wait_for_name(name, &conn, timeout)
    }

    fn wait_for_name(
        name: &str,
        conn: &Connection,
        timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult, Box<dyn std::error::Error>> {
        let dbus_proxy = DBusProxy::new(conn)?;

        // shortcut if name already exists
        if name_exists(name, &dbus_proxy)? {
            return Ok(WaitResult::Ok);
        }

        // Subscribe to NameOwnerChanged signals
        let signals = dbus_proxy.receive_name_owner_changed()?;

        let found = Arc::new(Mutex::new(false));
        let found_clone = found.clone();
        let target_name = name.to_owned();

        // Spawn a thread to listen for NameOwnerChanged signals
        let handle = std::thread::spawn(move || {
            for signal in signals {
                if let Ok(args) = signal.args()
                    && args.name.as_str() == target_name
                    && args.new_owner.is_some()
                {
                    *found_clone.lock().unwrap() = true;
                    return;
                }
            }
        });

        let start = std::time::Instant::now();
        let result = loop {
            if *found.lock().unwrap() {
                break WaitResult::Ok;
            }
            if let Some(timeout) = timeout
                && start.elapsed() >= timeout
            {
                break WaitResult::Timedout;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };

        // We don't join the signal thread - it will be cleaned up when the iterator
        // is dropped (which happens when the proxy/connection goes out of scope)
        drop(handle);

        Ok(result)
    }

    fn name_exists(name: &str, dbus_proxy: &DBusProxy) -> Result<bool, Box<dyn std::error::Error>> {
        let names = dbus_proxy.list_names()?;
        let target: OwnedBusName = BusName::from(WellKnownName::try_from(name)?).into();
        Ok(names.contains(&target))
    }

    #[test]
    fn test_dbus_wait() {
        let name = "org.test.WaitName".to_owned();
        let name2 = name.clone();

        std::thread::spawn(move || {
            // wait so the other thread has time to start waiting for the signal
            std::thread::sleep(std::time::Duration::from_secs(3));

            // request name on session bus
            let conn = Connection::session().unwrap();
            conn.request_name(WellKnownName::try_from(name2.as_str()).unwrap())
                .unwrap();
        });

        // wait for the name to be requested
        match wait_for_name_session_bus(&name, Some(std::time::Duration::from_millis(10_000)))
            .unwrap()
        {
            WaitResult::Ok => {
                println!("SUCCESS!!");
            }
            WaitResult::Timedout => {
                panic!("FAILED!!");
            }
        }

        // release the name after we are done
        let conn = Connection::session().unwrap();
        conn.release_name(WellKnownName::try_from(name.as_str()).unwrap())
            .unwrap();
    }
}
