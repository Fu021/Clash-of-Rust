//! Sort display indices without copying proxy snapshots or node names.
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, fmt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeSort {
    #[default]
    LatencyAscending,
    LatencyDescending,
    Name,
}

impl NodeSort {
    pub const ALL: [Self; 3] = [Self::LatencyAscending, Self::LatencyDescending, Self::Name];
}

impl fmt::Display for NodeSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LatencyAscending => "延迟从低到高",
            Self::LatencyDescending => "延迟从高到低",
            Self::Name => "名称排序",
        })
    }
}

/// Numeric runs compare by value without integer parsing or lowercase buffers.
pub fn natural_cmp(mut left: &str, mut right: &str) -> Ordering {
    while !left.is_empty() && !right.is_empty() {
        if left.as_bytes()[0].is_ascii_digit() && right.as_bytes()[0].is_ascii_digit() {
            let a = left.bytes().take_while(u8::is_ascii_digit).count();
            let b = right.bytes().take_while(u8::is_ascii_digit).count();
            let digits_a = left[..a].trim_start_matches('0');
            let digits_b = right[..b].trim_start_matches('0');
            let order = digits_a
                .len()
                .cmp(&digits_b.len())
                .then_with(|| digits_a.cmp(digits_b));
            if order != Ordering::Equal {
                return order;
            }
            left = &left[a..];
            right = &right[b..];
        } else {
            let a = left.chars().next().unwrap();
            let b = right.chars().next().unwrap();
            let order = a.to_lowercase().cmp(b.to_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            left = &left[a.len_utf8()..];
            right = &right[b.len_utf8()..];
        }
    }
    left.len().cmp(&right.len())
}

fn special(name: &str) -> u8 {
    match name {
        "DIRECT" => 0,
        "REJECT" => 1,
        "REJECT-DROP" => 2,
        "PASS" => 3,
        _ => 4,
    }
}

fn latency(delay: Option<u32>) -> (u8, u32) {
    match delay {
        Some(value) if value > 0 => (0, value),
        None => (1, 0),
        Some(_) => (2, 0),
    }
}

pub fn sorted_indices<T: AsRef<str>>(
    nodes: &[T],
    mode: NodeSort,
    delay: impl Fn(&str) -> Option<u32>,
    matches: impl Fn(&str) -> bool,
) -> Vec<usize> {
    let mut indices: Vec<_> = (0..nodes.len())
        .filter(|&i| matches(nodes[i].as_ref()))
        .collect();
    indices.sort_unstable_by(|&a, &b| {
        let left = nodes[a].as_ref();
        let right = nodes[b].as_ref();
        special(left).cmp(&special(right)).then_with(|| {
            let order = if mode == NodeSort::Name {
                Ordering::Equal
            } else {
                let (class_a, value_a) = latency(delay(left));
                let (class_b, value_b) = latency(delay(right));
                class_a.cmp(&class_b).then_with(|| {
                    if mode == NodeSort::LatencyDescending {
                        value_b.cmp(&value_a)
                    } else {
                        value_a.cmp(&value_b)
                    }
                })
            };
            order.then_with(|| natural_cmp(left, right)).then(a.cmp(&b))
        })
    });
    indices.shrink_to_fit();
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_names_handle_numbers_unicode_case_and_large_values() {
        assert!(natural_cmp("节点2", "节点10").is_lt());
        assert_eq!(natural_cmp("École02", "école2"), Ordering::Equal);
        assert!(natural_cmp("node999999999999999999999", "node1000000000000000000000").is_lt());
        assert!(natural_cmp("node2a", "node2b").is_lt());
    }

    #[test]
    fn descending_keeps_unknown_and_failures_last_and_special_options_first() {
        let names = [
            "node10", "node2", "unknown", "failed", "DIRECT", "REJECT", "node02",
        ];
        let delay = |name: &str| match name {
            "node10" => Some(80),
            "node2" | "node02" => Some(20),
            "failed" => Some(0),
            _ => None,
        };
        let order = |mode| sorted_indices(&names, mode, delay, |_| true);
        assert_eq!(order(NodeSort::LatencyAscending), [4, 5, 1, 6, 0, 2, 3]);
        assert_eq!(order(NodeSort::LatencyDescending), [4, 5, 0, 1, 6, 2, 3]);
        assert_eq!(order(NodeSort::Name), [4, 5, 3, 1, 6, 0, 2]);
        assert_eq!(
            sorted_indices(&names, NodeSort::Name, delay, |name| name
                .starts_with("node")),
            [1, 6, 0]
        );
    }

    #[test]
    fn filtering_then_sorting_preserves_global_order_across_pages() {
        let nodes: Vec<_> = (0..125).rev().map(|i| format!("node{i}")).collect();
        let ordered = sorted_indices(&nodes, NodeSort::Name, |_| None, |_| true);
        assert_eq!(nodes[ordered[59]], "node59");
        assert_eq!(nodes[ordered[60]], "node60");
        assert_eq!(nodes[ordered[124]], "node124");
    }
}
