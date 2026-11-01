//! Read-only earned reset credits. The reported count is authoritative;
//! missing/truncated detail rows never become a zero count or a guessed date.
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_AGE: Duration = Duration::from_secs(150);

#[derive(Clone, Debug)]
enum Expiry {
    At(i64),
    Never,
    Unknown,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    available: Option<u64>,
    expiries: Vec<Expiry>,
    fetched_at: Option<Instant>,
}
#[derive(Clone)]
pub struct View {
    pub title: String,
    pub value: String,
    pub count_text: String,
    pub expiry_text: String,
    pub details: Vec<String>,
}
fn cell() -> &'static Mutex<Snapshot> {
    static CELL: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(Snapshot::default()))
}
pub fn store(result: Option<&serde_json::Value>) {
    *cell().lock().expect("reset credit snapshot poisoned") =
        result.map(Snapshot::parse).unwrap_or_default();
}
pub fn latest() -> View {
    cell()
        .lock()
        .expect("reset credit snapshot poisoned")
        .view()
}
impl Snapshot {
    pub fn parse(result: &serde_json::Value) -> Self {
        let Some(credits) = result
            .get("rateLimitResetCredits")
            .filter(|v| v.is_object())
        else {
            return Self::default();
        };
        let available = credits.get("availableCount").and_then(|n| n.as_u64());
        let mut seen = HashSet::new();
        let mut expiries = Vec::new();
        if let Some(rows) = credits.get("credits").and_then(|v| v.as_array()) {
            for row in rows {
                if row.get("status").and_then(|s| s.as_str()) != Some("available") {
                    continue;
                }
                let Some(id) = row
                    .get("id")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                if !seen.insert(id) {
                    continue;
                }
                let expiry = match row.get("expiresAt") {
                    // The official schema explicitly defines null as no expiry.
                    Some(serde_json::Value::Null) => Expiry::Never,
                    Some(value) => value
                        .as_i64()
                        .filter(|n| (0..=253402300799).contains(n))
                        .filter(|n| chrono::DateTime::from_timestamp(*n, 0).is_some())
                        .map(Expiry::At)
                        .unwrap_or(Expiry::Unknown),
                    None => Expiry::Unknown,
                };
                expiries.push(expiry);
            }
        }
        Self {
            available,
            expiries,
            fetched_at: Some(Instant::now()),
        }
    }
    pub fn view(&self) -> View {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.view_at(now, Instant::now())
    }
    fn view_at(&self, now: i64, instant: Instant) -> View {
        let unknown = || View {
            title: "Resets do Codex:".into(),
            value: "Não informado".into(),
            count_text: "Resets da conta: informação indisponível".into(),
            expiry_text: "Validade: informação indisponível".into(),
            details: vec!["A conta não forneceu uma leitura atual dos resets".into()],
        };
        if self
            .fetched_at
            .is_none_or(|at| instant.saturating_duration_since(at) > MAX_AGE)
        {
            return unknown();
        }
        let Some(count) = self.available else {
            return unknown();
        };
        let mut title = if count == 1 {
            "Reset: 1 disponível".into()
        } else {
            format!("Resets: {count} disponíveis")
        };
        let mut count_text = format!("Resets disponíveis na conta: {count}");
        if count == 0 {
            return View {
                title,
                value: "Nenhum disponível".into(),
                count_text,
                expiry_text: "Validade: nenhum reset disponível".into(),
                details: vec!["Nenhum reset disponível na conta".into()],
            };
        }
        let mut groups: BTreeMap<i64, u64> = BTreeMap::new();
        let mut never = 0_u64;
        let mut unknown_count = count.saturating_sub(self.expiries.len() as u64);
        let mut expired = false;
        for expiry in &self.expiries {
            match expiry {
                Expiry::At(at) if *at <= now => expired = true,
                Expiry::At(at) => *groups.entry(*at).or_default() += 1,
                Expiry::Never => never += 1,
                Expiry::Unknown => unknown_count += 1,
            }
        }
        let mut details = vec![count_text.clone()];
        for (at, n) in &groups {
            let date = chrono::DateTime::from_timestamp(*at, 0)
                .unwrap()
                .with_timezone(&chrono::Local);
            details.push(format!(
                "{n} {}: vence em {} (hora local)",
                if *n == 1 { "reset" } else { "resets" },
                date.format("%d/%m/%Y às %H:%M")
            ));
        }
        if never > 0 {
            details.push(format!(
                "{never} {} sem vencimento",
                if never == 1 { "reset" } else { "resets" }
            ));
        }
        if unknown_count > 0 {
            details.push(format!(
                "{unknown_count} {} com validade não informada",
                if unknown_count == 1 {
                    "reset"
                } else {
                    "resets"
                }
            ));
        }
        let partial = unknown_count > 0 || self.expiries.len() as u64 != count;
        let (value, expiry_text) = if expired {
            title = "Resets: consultando".into();
            count_text = format!("Resets informados: {count} (validade a confirmar)");
            details.push("Há data já vencida na última leitura; aguarde a próxima consulta".into());
            (
                "Validade a confirmar".into(),
                "Validade: dados precisam de nova consulta".into(),
            )
        } else if let Some((at, _)) = groups.first_key_value() {
            let date = chrono::DateTime::from_timestamp(*at, 0)
                .unwrap()
                .with_timezone(&chrono::Local);
            (
                format!(
                    "Vence {}{}",
                    date.format("%d/%m %H:%M"),
                    if partial { "*" } else { "" }
                ),
                format!(
                    "{}: {}",
                    if partial {
                        "Vencimento informado*"
                    } else {
                        "Próximo vencimento"
                    },
                    date.format("%d/%m/%Y %H:%M")
                ),
            )
        } else if never == count && !partial {
            (
                "Sem vencimento".into(),
                "Validade: resets sem vencimento".into(),
            )
        } else {
            (
                "Validade não informada".into(),
                "Validade: detalhes não informados pela conta".into(),
            )
        };
        if partial {
            details.push("* Datas parciais; o total informado pela conta foi preservado".into());
        }
        View {
            title,
            value,
            count_text,
            expiry_text,
            details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(count: serde_json::Value, rows: serde_json::Value) -> Snapshot {
        Snapshot::parse(
            &serde_json::json!({"rateLimitResetCredits":{"availableCount":count,"credits":rows}}),
        )
    }
    fn row(id: &str, status: &str, expiry: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"id":id,"status":status,"expiresAt":expiry})
    }
    #[test]
    fn missing_is_unknown_while_reported_zero_is_zero() {
        assert_eq!(
            Snapshot::parse(&serde_json::json!({})).view().value,
            "Não informado"
        );
        let zero = snapshot(serde_json::json!(0), serde_json::json!([])).view();
        assert!(zero.count_text.ends_with(": 0"));
        assert_eq!(zero.value, "Nenhum disponível");
    }
    #[test]
    fn authoritative_count_is_not_replaced_by_the_number_of_details() {
        let s = snapshot(
            serde_json::json!(8),
            serde_json::json!([row("a", "available", serde_json::json!(2000000000_i64))]),
        );
        let v = s.view_at(1800000000, Instant::now());
        assert!(v.title.contains("8 disponíveis"));
        assert!(v.value.ends_with('*'));
        assert!(v
            .details
            .iter()
            .any(|s| s.contains("7 resets com validade não informada")));
        assert_eq!(
            snapshot(serde_json::json!(2), serde_json::Value::Null)
                .view()
                .value,
            "Validade não informada"
        );
    }
    #[test]
    fn only_available_distinct_credits_contribute_expirations() {
        let s = snapshot(
            serde_json::json!(2),
            serde_json::json!([
                row("old", "redeemed", serde_json::json!(1800000001_i64)),
                row("a", "available", serde_json::json!(2000000000_i64)),
                row("a", "available", serde_json::json!(2000000000_i64)),
                row("b", "available", serde_json::json!(1900000000_i64))
            ]),
        );
        let v = s.view_at(1800000000, Instant::now());
        assert!(v.expiry_text.starts_with("Próximo vencimento:"));
        assert_eq!(v.details.len(), 3);
        assert!(!v.value.ends_with('*'));
        let earliest = chrono::DateTime::from_timestamp(1900000000, 0)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert!(v
            .value
            .contains(&earliest.format("%d/%m %H:%M").to_string()));
    }
    #[test]
    fn explicit_null_means_no_expiry_but_absent_field_is_unknown() {
        let never = snapshot(
            serde_json::json!(1),
            serde_json::json!([row("a", "available", serde_json::Value::Null)]),
        );
        assert_eq!(never.view().value, "Sem vencimento");
        let unknown = snapshot(
            serde_json::json!(1),
            serde_json::json!([{"id":"a","status":"available"}]),
        );
        assert_eq!(unknown.view().value, "Validade não informada");
    }
    #[test]
    fn expired_or_stale_data_is_not_presented_as_a_valid_future_deadline() {
        let s = snapshot(
            serde_json::json!(1),
            serde_json::json!([row("a", "available", serde_json::json!(100))]),
        );
        assert_eq!(s.view_at(200, Instant::now()).value, "Validade a confirmar");
        assert_eq!(
            s.view_at(0, s.fetched_at.unwrap() + MAX_AGE + Duration::from_secs(1))
                .value,
            "Não informado"
        );
    }
}
