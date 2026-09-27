use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    Skip,
}

/// One row of the connect checklist shown in the Brokers panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    #[serde(default)]
    pub detail: String,
}

#[derive(Default)]
pub(crate) struct Checklist {
    items: Vec<Check>,
}

impl Checklist {
    pub fn push(&mut self, id: &str, label: &str, status: CheckStatus, detail: impl Into<String>) {
        self.items.push(Check {
            id: id.into(),
            label: label.into(),
            status,
            detail: detail.into(),
        });
    }

    pub fn ok(&mut self, id: &str, label: &str, detail: impl Into<String>) {
        self.push(id, label, CheckStatus::Ok, detail);
    }

    pub fn warn(&mut self, id: &str, label: &str, detail: impl Into<String>) {
        self.push(id, label, CheckStatus::Warn, detail);
    }

    pub fn fail(&mut self, id: &str, label: &str, detail: impl Into<String>) {
        self.push(id, label, CheckStatus::Fail, detail);
    }

    /// Marks the steps that could not run because an earlier one failed.
    pub fn skip(&mut self, steps: &[(&str, &str)]) {
        for (id, label) in steps {
            self.push(id, label, CheckStatus::Skip, "");
        }
    }

    pub fn extend(&mut self, checks: impl IntoIterator<Item = Check>) {
        self.items.extend(checks);
    }

    pub fn has_fail(&self) -> bool {
        self.items.iter().any(|c| c.status == CheckStatus::Fail)
    }

    pub fn into_vec(self) -> Vec<Check> {
        self.items
    }
}

/// Clock skew against the venue. Signed requests carry a 5 s receive window.
pub(crate) fn clock(list: &mut Checklist, skew_ms: i64) {
    let secs = skew_ms as f64 / 1000.0;
    let detail = format!("off by {secs:+.2} s");
    if skew_ms.abs() < 1_000 {
        list.ok("clock", "Clock in sync", detail);
    } else if skew_ms.abs() < 4_000 {
        list.warn("clock", "Clock in sync", format!("{detail}; sync the system clock"));
    } else {
        list.fail(
            "clock",
            "Clock in sync",
            format!("{detail}; signed requests will be rejected, sync the system clock"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_thresholds() {
        let status = |ms| {
            let mut l = Checklist::default();
            clock(&mut l, ms);
            l.into_vec()[0].status
        };
        assert_eq!(status(-900), CheckStatus::Ok);
        assert_eq!(status(2_500), CheckStatus::Warn);
        assert_eq!(status(-6_000), CheckStatus::Fail);
    }
}
