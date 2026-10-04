use std::fmt;
use windows_registry::CURRENT_USER;
use windows_registry::Type;

const VALUE_NAME: &str = "Data";

/// Registry value that stores one Night Light payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Settings,
    State,
}

impl Key {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Settings => {
                r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings"
            }
            Self::State => {
                r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate"
            }
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Settings => "Night Light settings",
            Self::State => "Night Light state",
        })
    }
}

/// Byte storage for the Night Light registry values.
pub(crate) trait Store {
    fn read(&self, key: Key) -> windows_result::Result<Vec<u8>>;
    fn write(&self, key: Key, data: &[u8]) -> windows_result::Result<()>;
}

/// Store backed by the current user's registry hive.
#[derive(Debug)]
pub(crate) struct RegistryStore;

impl Store for RegistryStore {
    fn read(&self, key: Key) -> windows_result::Result<Vec<u8>> {
        CURRENT_USER
            .options()
            .read()
            .open(key.path())?
            .get_bytes(VALUE_NAME)
    }

    fn write(&self, key: Key, data: &[u8]) -> windows_result::Result<()> {
        CURRENT_USER
            .options()
            .write()
            .open(key.path())?
            .set_bytes(VALUE_NAME, Type::Bytes, data)
    }
}

#[cfg(test)]
pub(crate) mod memory {
    use super::Key;
    use super::Store;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use windows_result::WIN32_ERROR;

    const ERROR_FILE_NOT_FOUND: WIN32_ERROR = WIN32_ERROR(2);

    /// In-memory store that records the keys written, in order.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct MemoryStore(Arc<Mutex<Inner>>);

    #[derive(Debug, Default)]
    struct Inner {
        values: HashMap<Key, Vec<u8>>,
        writes: Vec<Key>,
    }

    impl MemoryStore {
        pub(crate) fn with(values: &[(Key, &[u8])]) -> Self {
            let store = Self::default();
            store.lock().values = values
                .iter()
                .map(|(key, data)| (*key, data.to_vec()))
                .collect();
            store
        }

        pub(crate) fn value(&self, key: Key) -> Option<Vec<u8>> {
            self.lock().values.get(&key).cloned()
        }

        pub(crate) fn writes(&self) -> Vec<Key> {
            self.lock().writes.clone()
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
            self.0.lock().expect("memory store lock is not poisoned")
        }
    }

    impl Store for MemoryStore {
        fn read(&self, key: Key) -> windows_result::Result<Vec<u8>> {
            self.value(key).ok_or_else(|| ERROR_FILE_NOT_FOUND.into())
        }

        fn write(&self, key: Key, data: &[u8]) -> windows_result::Result<()> {
            let mut inner = self.lock();
            inner.values.insert(key, data.to_vec());
            inner.writes.push(key);
            Ok(())
        }
    }
}
