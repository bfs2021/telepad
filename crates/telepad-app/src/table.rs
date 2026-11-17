//! Overview table: which rows to show and in what order. Pure functions, so
//! the view only draws and sorting is unit-tested here.

use std::cmp::Ordering;
use std::net::IpAddr;
use std::time::Instant;

use telepad_core::model::{Disk, HostReport, SensorKind};

use crate::state::{HostState, Model, matches_filter};

/// A sortable overview column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Status,
    Host,
    Description,
    Address,
    Os,
    Cpu,
    Memory,
    Disk,
    Temp,
    Docker,
    Updated,
}

impl SortKey {
    /// Numbers start with the largest (busiest, hottest, newest) first.
    pub fn starts_descending(self) -> bool {
        matches!(
            self,
            Self::Cpu | Self::Memory | Self::Disk | Self::Temp | Self::Docker | Self::Updated
        )
    }
}

/// Current overview order; `None` = the inventory order from the database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sort {
    pub key: Option<SortKey>,
    pub descending: bool,
}

impl Sort {
    /// Header click: new column → its natural direction; same column →
    /// reversed; third click → back to inventory order.
    pub fn clicked(self, key: SortKey) -> Self {
        match self.key {
            Some(k) if k == key && self.descending != key.starts_descending() => Self::default(),
            Some(k) if k == key => Self {
                key: Some(key),
                descending: !self.descending,
            },
            _ => Self {
                key: Some(key),
                descending: key.starts_descending(),
            },
        }
    }

    /// `Some(descending)` when `key` is the sort column (for the header arrow).
    pub fn direction(self, key: SortKey) -> Option<bool> {
        (self.key == Some(key)).then_some(self.descending)
    }
}

/// Indices into `m.hosts` that match `filter`, in `sort` order (stable; hosts
/// without a value for the sort column always come last).
pub fn overview_rows(m: &Model, filter: &str, sort: Sort) -> Vec<usize> {
    let mut rows: Vec<usize> = m
        .hosts
        .iter()
        .enumerate()
        .filter(|(_, h)| matches_filter(filter, h))
        .map(|(i, _)| i)
        .collect();
    if let Some(key) = sort.key {
        rows.sort_by(|&a, &b| {
            let (a, b) = (sort_value(&m.hosts[a], key), sort_value(&m.hosts[b], key));
            match (a, b) {
                (Some(a), Some(b)) => {
                    let o = a.cmp(&b);
                    if sort.descending { o.reverse() } else { o }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        });
    }
    rows
}

#[derive(Debug)]
enum Value {
    Num(f64),
    Text(String),
    At(Instant),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Value {}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => a.total_cmp(b),
            (Value::Text(a), Value::Text(b)) => a.cmp(b),
            (Value::At(a), Value::At(b)) => a.cmp(b),
            // Only Address mixes kinds: IPv4 numbers before names.
            (a, b) => rank(a).cmp(&rank(b)),
        }
    }
}

fn rank(v: &Value) -> u8 {
    match v {
        Value::Num(_) => 0,
        Value::Text(_) => 1,
        Value::At(_) => 2,
    }
}

fn text(s: &str) -> Option<Value> {
    let s = s.trim();
    (!s.is_empty()).then(|| Value::Text(s.to_lowercase()))
}

fn sort_value(h: &HostState, key: SortKey) -> Option<Value> {
    let r = h.report.as_ref();
    match key {
        // down, then unknown/loading, then up
        SortKey::Status => Some(Value::Num(match h.online() {
            Some(false) => 0.0,
            None => 1.0,
            Some(true) => 2.0,
        })),
        SortKey::Host => text(&h.entry.alias),
        SortKey::Description => text(&h.entry.note),
        SortKey::Address => address_value(&h.entry.hostname).or_else(|| text(&h.entry.alias)),
        SortKey::Os => r.and_then(|r| text(&r.os_pretty)),
        SortKey::Cpu => r
            .and_then(|r| r.cpu_usage_pct)
            .map(|v| Value::Num(v.into())),
        SortKey::Memory => r
            .and_then(|r| r.memory.as_ref())
            .map(|m| Value::Num(m.used_pct().into())),
        SortKey::Disk => r
            .and_then(fullest_disk)
            .map(|d| Value::Num(d.used_pct().into())),
        SortKey::Temp => r.and_then(hottest).map(|t| Value::Num(t.into())),
        SortKey::Docker => r
            .filter(|r| r.docker.unavailable.is_none())
            .map(|r| Value::Num(r.docker.containers.iter().filter(|c| c.running()).count() as f64)),
        // Most recent first when descending: newer Instants are larger.
        SortKey::Updated => h.updated.map(Value::At),
    }
}

/// IPv4/IPv6 addresses sort numerically (10.0.0.9 before 10.0.0.10).
fn address_value(s: &str) -> Option<Value> {
    let s = s.trim();
    match s.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => Some(Value::Num(f64::from(u32::from(v4)))),
        Ok(IpAddr::V6(v6)) => Some(Value::Text(format!("~{:032x}", u128::from(v6)))),
        Err(_) => text(s),
    }
}

/// The mount with the highest usage.
pub fn fullest_disk(r: &HostReport) -> Option<&Disk> {
    r.disks
        .iter()
        .max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()))
}

/// Highest temperature reading, in °C.
pub fn hottest(r: &HostReport) -> Option<f32> {
    r.sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .map(|s| s.value)
        .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))))
}
