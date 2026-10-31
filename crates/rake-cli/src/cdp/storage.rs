//! Chrome local/session storage is a bounded projection of explicit native stores.

use super::{
    TIMEOUT,
    wire::{Error, Event},
};

use crate::terminal::{exchange, query};
use rake_client::local::Instance;

use rake_protocol::{
    control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request},
    storage::{Area, Descriptor, Edit, Page, Read},
};

use serde_json::{Value, json};
use std::collections::BTreeMap;

const ORIGIN: &str = "rake://runtime";
const KEY: &str = "rake://runtime/";
const MAX_ROWS: usize = 256;
const MAX_BYTES: usize = 384 * 1024;

type Rows = BTreeMap<String, String>;
type Stamp = Vec<(String, u64)>;

#[derive(Default)]
pub(super) struct Storage {
    watched: BTreeMap<bool, (Stamp, Rows)>,
}

impl Storage {
    pub(super) fn recover_mutation(&mut self, instance: &Instance, params: &Value) -> Vec<Event> {
        let Ok(local) = area(params) else {
            return Vec::new();
        };

        let Ok((actual, _)) = self.handle(instance, "DOMStorage.getDOMStorageItems", params) else {
            return Vec::new();
        };

        let Some(key) = params["key"].as_str() else {
            // Chrome also clears the table optimistically on its Clear All button.
            return actual["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|entry| {
                    Event::new(
                        "DOMStorage.domStorageItemAdded",
                        json!({"storageId":id(local),"key":entry[0],"newValue":entry[1]}),
                    )
                })
                .collect();
        };

        let value = actual["entries"]
            .as_array()
            .and_then(|entries| entries.iter().find(|entry| entry[0] == key))
            .and_then(|entry| entry[1].as_str());

        // Chrome can retain optimistic cell text when an update equals its
        // pre-edit cached value. Recreate this projection row from actual data.
        let mut events = vec![Event::new(
            "DOMStorage.domStorageItemRemoved",
            json!({"storageId":id(local),"key":key}),
        )];

        if let Some(value) = value {
            events.push(Event::new(
                "DOMStorage.domStorageItemAdded",
                json!({"storageId":id(local),"key":key,"newValue":value}),
            ));
        }

        events
    }

    pub(super) fn clear(&mut self) {
        self.watched.clear();
    }

    pub(super) fn reset(&mut self) -> Vec<Event> {
        self.watched
            .iter_mut()
            .map(|(local, state)| {
                *state = (Vec::new(), Rows::new());

                Event::new(
                    "DOMStorage.domStorageItemsCleared",
                    json!({"storageId":id(*local)}),
                )
            })
            .collect()
    }

    pub(super) fn storage_key(params: &Value) -> Result<Value, Error> {
        if params["frameId"] != "rake" {
            return Err(Error::new(
                -32602,
                "Only the Rake runtime frame has application storage",
            ));
        }

        Ok(json!({"storageKey":KEY}))
    }

    pub(super) fn handle(
        &mut self,
        instance: &Instance,
        method: &str,
        params: &Value,
    ) -> Result<(Value, Vec<Event>), Error> {
        if method == "Rake.getStorage" {
            return Ok((json!({"stores":catalog(instance)?}), Vec::new()));
        }

        let local = area(params)?;
        let stores = catalog(instance)?;

        if method == "DOMStorage.getDOMStorageItems" {
            let rows = rows(instance, &stores, local)?;

            let entries: Vec<_> = rows
                .iter()
                .map(|(key, value)| json!([key, value]))
                .collect();

            self.watched.insert(local, (stamp(&stores, local), rows));

            return Ok((json!({"entries":entries}), Vec::new()));
        }

        if method == "DOMStorage.clear" {
            return Err(Error::new(
                -32601,
                "Bulk clearing registered application stores is unavailable; remove individual writable entries",
            ));
        }

        let key = params["key"]
            .as_str()
            .ok_or_else(|| Error::new(-32602, "key required"))?;

        let (name, key) = key
            .split_once('/')
            .filter(|(_, key)| !key.is_empty())
            .ok_or_else(|| Error::new(-32602, "Keys use registered-store/key"))?;

        if !stores
            .iter()
            .any(|store| store.name == name && matches_area(store.area, local) && store.writable)
        {
            return Err(Error::new(
                -32000,
                "Unknown or read-only storage registration",
            ));
        }

        let value = if method == "DOMStorage.setDOMStorageItem" {
            Some(
                params["value"]
                    .as_str()
                    .ok_or_else(|| Error::new(-32602, "JSON value required"))?
                    .to_owned(),
            )
        } else if method == "DOMStorage.removeDOMStorageItem" {
            None
        } else {
            return Err(Error::new(-32601, "Unsupported storage method"));
        };

        let reply = exchange(
            instance,
            &Request {
                protocol: PROTOCOL_VERSION,
                runtime_id: instance.info.runtime_id,
                key: Some(uuid::Uuid::new_v4().to_string()),
                message: Message::Command(Command::EditStorage {
                    edit: Edit {
                        store: name.into(),
                        key: key.into(),
                        value,
                    },
                }),
            },
            TIMEOUT,
        )
        .map_err(error)?;

        if !matches!(reply, Reply::Acknowledged) {
            return Err(Error::new(-32000, "Unexpected storage mutation reply"));
        }

        // A committed write must not become a reported failure just because a
        // follow-up refresh failed. Polling/manual refresh can retry the read.
        let events = self.poll(instance).unwrap_or_default();

        Ok((json!({}), events))
    }

    pub(super) fn poll(&mut self, instance: &Instance) -> Result<Vec<Event>, Error> {
        if self.watched.is_empty() {
            return Ok(Vec::new());
        }

        let stores = catalog(instance)?;
        let mut events = Vec::new();

        for (local, state) in &mut self.watched {
            let stamp = stamp(&stores, *local);

            if stamp == state.0 {
                continue;
            }

            let current = rows(instance, &stores, *local)?;

            events.extend(changes(*local, &state.1, &current));
            *state = (stamp, current);
        }

        Ok(events)
    }
}

fn catalog(instance: &Instance) -> Result<Vec<Descriptor>, Error> {
    if !instance.info.queries.iter().any(|query| query == "storage") {
        return Ok(Vec::new());
    }

    let data = read(instance, Read::Stores)?;
    let stores: Vec<Descriptor> = serde_json::from_value(data).map_err(error)?;

    if stores.len() > 32 {
        return Err(Error::new(-32000, "Invalid storage catalog"));
    }

    Ok(stores)
}

fn read(instance: &Instance, read: Read) -> Result<Value, Error> {
    match query(instance, Query::Storage { read }, TIMEOUT).map_err(error)? {
        Reply::Data(data) => Ok(data),
        _ => Err(Error::new(-32000, "Unexpected storage reply")),
    }
}

fn rows(instance: &Instance, stores: &[Descriptor], local: bool) -> Result<Rows, Error> {
    let mut rows = Rows::new();
    let mut bytes = 0;

    for store in stores
        .iter()
        .filter(|store| matches_area(store.area, local))
    {
        let mut after = None;

        // Includes the final empty page when exactly MAX_ROWS keys were read.
        for page_index in 0..=MAX_ROWS / 128 {
            let page: Page = serde_json::from_value(read(
                instance,
                Read::Entries {
                    store: store.name.clone(),
                    after: after.clone(),
                    limit: 128,
                },
            )?)
            .map_err(error)?;

            if page.revision != store.revision {
                return Err(Error::new(
                    -32000,
                    "Storage changed during refresh; refresh again",
                ));
            }

            for (key, value) in page.entries {
                let key = format!("{}/{key}", store.name);

                bytes += key.len() + value.len();

                if rows.len() >= MAX_ROWS || bytes > MAX_BYTES {
                    return Err(Error::new(
                        -32000,
                        "Chrome storage projection exceeds 256 rows / 384 KiB; register a smaller store",
                    ));
                }

                if rows.insert(key, value).is_some() {
                    return Err(Error::new(-32000, "Duplicate storage key in page"));
                }
            }

            match page.next {
                None => break,
                Some(next) if after.as_ref().is_none_or(|after| next > *after) => {
                    if page_index == MAX_ROWS / 128 {
                        return Err(Error::new(
                            -32000,
                            "Chrome storage projection requires too many pages; register a smaller store",
                        ));
                    }

                    after = Some(next)
                }
                _ => return Err(Error::new(-32000, "Storage cursor did not advance")),
            }
        }
    }

    Ok(rows)
}

fn stamp(stores: &[Descriptor], local: bool) -> Stamp {
    stores
        .iter()
        .filter(|store| matches_area(store.area, local))
        .map(|store| (store.name.clone(), store.revision))
        .collect()
}

fn matches_area(area: Area, local: bool) -> bool {
    matches!(area, Area::Local) == local
}

fn id(local: bool) -> Value {
    json!({"storageKey":KEY,"isLocalStorage":local})
}

fn area(params: &Value) -> Result<bool, Error> {
    let storage = &params["storageId"];

    if storage.get("storageKey").is_some_and(|key| key != KEY)
        || storage
            .get("securityOrigin")
            .is_some_and(|origin| origin != ORIGIN)
        || storage.get("storageKey").is_none() && storage.get("securityOrigin").is_none()
    {
        return Err(Error::new(
            -32602,
            "Storage identifier must target rake://runtime",
        ));
    }

    storage["isLocalStorage"]
        .as_bool()
        .ok_or_else(|| Error::new(-32602, "isLocalStorage must be boolean"))
}

fn changes(local: bool, previous: &Rows, current: &Rows) -> Vec<Event> {
    let mut events = Vec::new();

    for key in previous.keys().filter(|key| !current.contains_key(*key)) {
        events.push(Event::new(
            "DOMStorage.domStorageItemRemoved",
            json!({"storageId":id(local),"key":key}),
        ));
    }

    for (key, value) in current {
        match previous.get(key) {
            Some(old) if old != value => events.push(Event::new(
                "DOMStorage.domStorageItemUpdated",
                json!({"storageId":id(local),"key":key,"oldValue":old,"newValue":value}),
            )),
            None => events.push(Event::new(
                "DOMStorage.domStorageItemAdded",
                json!({"storageId":id(local),"key":key,"newValue":value}),
            )),
            _ => {}
        }
    }

    events
}

fn error(error: impl std::fmt::Display) -> Error {
    Error::new(-32000, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_identifiers_and_changed_value_events() {
        assert!(
            area(&json!({"storageId":{"securityOrigin":ORIGIN,"isLocalStorage":true}})).unwrap()
        );
        assert!(!area(&json!({"storageId":id(false)})).unwrap());

        for storage in [
            json!({"isLocalStorage":true}),
            json!({"storageKey":"https://other/","isLocalStorage":true}),
            json!({"storageKey":KEY,"isLocalStorage":1}),
        ] {
            assert!(area(&json!({"storageId":storage})).is_err());
        }

        let old = [
            ("prefs/a".into(), "1".into()),
            ("prefs/b".into(), "2".into()),
        ]
        .into();

        let new = [
            ("prefs/a".into(), "3".into()),
            ("prefs/c".into(), "4".into()),
        ]
        .into();

        let events = changes(true, &old, &new);

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].method, "DOMStorage.domStorageItemRemoved");
        assert_eq!(events[1].params["oldValue"], "1");
        assert_eq!(events[2].method, "DOMStorage.domStorageItemAdded");
    }
}
