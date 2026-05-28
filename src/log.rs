use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const MAX_EVENTS: usize = 128;

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub fields: Vec<(String, String)>,
}

struct Store {
    events: VecDeque<Event>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

pub struct Span {
    name: &'static str,
    start: Instant,
    fields: Vec<(String, String)>,
}

pub fn timer(name: &'static str) -> Span {
    Span {
        name,
        start: Instant::now(),
        fields: Vec::new(),
    }
}

pub fn add_event(name: &str, fields: &[(&str, String)]) {
    let event = Event {
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    };

    let store = STORE.get_or_init(|| {
        Mutex::new(Store {
            events: VecDeque::with_capacity(MAX_EVENTS),
        })
    });

    if let Ok(mut store) = store.lock() {
        if store.events.len() == MAX_EVENTS {
            store.events.pop_front();
        }
        store.events.push_back(event);
    }
}

pub fn recent_events(limit: usize) -> Vec<Event> {
    let Some(store) = STORE.get() else {
        return Vec::new();
    };

    let Ok(store) = store.lock() else {
        return Vec::new();
    };

    store
        .events
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
}

impl Span {
    pub fn field(&mut self, key: &str, value: impl ToString) {
        self.fields.push((key.to_string(), value.to_string()));
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let mut fields = Vec::with_capacity(self.fields.len() + 1);
        fields.push(("elapsed_ms", self.start.elapsed().as_millis().to_string()));
        for (key, value) in &self.fields {
            fields.push((key.as_str(), value.clone()));
        }
        add_event(self.name, &fields);
    }
}
