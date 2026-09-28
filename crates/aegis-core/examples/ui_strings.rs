//! Prints every user-facing English string the core sends to the window (for translations).
use aegis_core::{
    bounce::{param_specs, FEATURES, KINDS, SESSIONS},
    strategy, BrokerId,
};

fn main() {
    let mut v: Vec<String> = Vec::new();
    for s in param_specs() {
        v.extend([s.label, s.help, s.group.to_string()]);
    }
    for f in FEATURES {
        v.extend([f.label.to_string(), f.help.to_string(), f.group.to_string()]);
    }
    v.extend(KINDS.iter().map(|k| k.label().to_string()));
    v.extend(SESSIONS.iter().map(|s| s.1.to_string()));
    for s in strategy::catalog() {
        v.extend([s.name.to_string(), s.summary.to_string()]);
    }
    for b in BrokerId::ALL {
        let i = b.info();
        v.extend(i.requirements.iter().map(|r| r.to_string()));
        for f in i.fields {
            v.extend([f.label.to_string(), f.hint.to_string()]);
        }
    }
    v.retain(|s| !s.trim().is_empty());
    v.sort();
    v.dedup();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
}
