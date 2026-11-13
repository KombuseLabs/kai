//! fence's ledger, read: one JSON line per decision, of which kai counts
//! those with `"kind": "net"` (`host`, `port`, `decision`). Lines of
//! another kind are passed over; a line kai cannot read is an error with
//! its number, never a count of 0.

use std::collections::BTreeMap;

/// The ledger's name in the run's directory.
pub(crate) const LEDGER: &str = "proxy.jsonl";

/// What went through an egress, counted from its ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Connections let through because a rule allowed them.
    pub allowed: u64,
    /// Connections refused.
    pub refused: u64,
    /// Connections let through in learn mode that no rule allowed.
    pub would_deny: u64,
    /// Each `host:port` refused, with how often.
    pub refused_targets: BTreeMap<String, u64>,
    /// Each `host:port` let through in learn mode that no rule allowed,
    /// with how often: what a policy would have to add.
    pub would_deny_targets: BTreeMap<String, u64>,
}

/// Counts the ledger's `net` lines. Fails with the number of the first
/// line kai cannot read, and what is wrong with it.
pub(crate) fn summarize(text: &str) -> Result<Summary, (usize, String)> {
    let mut summary = Summary::default();
    for (i, line) in text.lines().enumerate() {
        let number = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| (number, format!("not JSON: {e}")))?;
        if value.get("kind").and_then(serde_json::Value::as_str) != Some("net") {
            continue;
        }
        let field = |name: &str| {
            value
                .get(name)
                .ok_or_else(|| (number, format!("a net line without {name:?}")))
        };
        let host = field("host")?
            .as_str()
            .ok_or_else(|| (number, "a host that is not text".to_owned()))?;
        let port = field("port")?
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| (number, "a port that is not a port".to_owned()))?;
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        match field("decision")?.as_str() {
            Some("allow") => summary.allowed += 1,
            Some("deny") => {
                summary.refused += 1;
                *summary.refused_targets.entry(target).or_default() += 1;
            }
            Some("would-deny") => {
                summary.would_deny += 1;
                *summary.would_deny_targets.entry(target).or_default() += 1;
            }
            other => return Err((number, format!("a decision kai does not know: {other:?}"))),
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_the_net_lines() {
        let ledger = r#"{"v":1,"ts":"2026-11-13T10:00:00Z","kind":"net","host":"index.crates.io","port":443,"via":"connect","decision":"allow"}
{"v":1,"ts":"2026-11-13T10:00:01Z","kind":"net","host":"example.com","port":443,"via":"connect","decision":"deny"}
{"v":1,"ts":"2026-11-13T10:00:02Z","kind":"net","host":"example.com","port":443,"via":"http","decision":"deny","error":"non-public address 127.0.0.1"}

{"v":1,"ts":"2026-11-13T10:00:03Z","kind":"net","host":"::1","port":8080,"via":"connect","decision":"would-deny"}
{"v":1,"ts":"2026-11-13T10:00:04Z","kind":"run","exit":0}
"#;
        let summary = summarize(ledger).unwrap();
        assert_eq!(summary.allowed, 1);
        assert_eq!(summary.refused, 2);
        assert_eq!(summary.would_deny, 1);
        assert_eq!(
            summary.refused_targets,
            BTreeMap::from([("example.com:443".to_owned(), 2)])
        );
        assert_eq!(
            summary.would_deny_targets,
            BTreeMap::from([("[::1]:8080".to_owned(), 1)])
        );
        assert_eq!(summarize("").unwrap(), Summary::default());
    }

    #[test]
    fn a_line_kai_cannot_read_is_an_error_with_its_number() {
        let allow = r#"{"kind":"net","host":"a.example","port":443,"decision":"allow"}"#;
        for (bad, says) in [
            ("not json", "not JSON"),
            (
                r#"{"kind":"net","port":443,"decision":"allow"}"#,
                "\"host\"",
            ),
            (
                r#"{"kind":"net","host":"a","port":70000,"decision":"allow"}"#,
                "port",
            ),
            (r#"{"kind":"net","host":"a","port":443}"#, "\"decision\""),
            (
                r#"{"kind":"net","host":"a","port":443,"decision":"maybe"}"#,
                "maybe",
            ),
        ] {
            let (line, reason) = summarize(&format!("{allow}\n{bad}\n")).unwrap_err();
            assert_eq!(line, 2, "{bad}");
            assert!(reason.contains(says), "{bad}: {reason}");
        }
    }
}
