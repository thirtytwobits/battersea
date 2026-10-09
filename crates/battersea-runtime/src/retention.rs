//! Encoded-payload accounting shared by mailboxes and application-owned buffers.
use serde::Serialize;
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub items: usize,
    pub bytes: usize,
}
#[derive(Debug)]
struct Inner {
    limit: Usage,
    node_limit: usize,
    used: Usage,
    nodes: HashMap<String, usize>,
}
#[derive(Debug, Clone)]
pub struct RetentionBudget(Arc<Mutex<Inner>>);
impl RetentionBudget {
    pub fn new(items: usize, bytes: usize, node_bytes: usize) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            limit: Usage { items, bytes },
            node_limit: node_bytes,
            used: Usage::default(),
            nodes: HashMap::new(),
        })))
    }
    pub fn usage(&self) -> Usage {
        self.0.lock().unwrap().used
    }
    pub(crate) fn reserve(
        &self,
        items: usize,
        bytes: usize,
        node: Option<&str>,
    ) -> Result<Reservation, String> {
        let mut inner = self.0.lock().unwrap();
        let node_used = node
            .and_then(|id| inner.nodes.get(id))
            .copied()
            .unwrap_or(0);
        if items > inner.limit.items - inner.used.items
            || bytes > inner.limit.bytes - inner.used.bytes
            || (node.is_some() && bytes > inner.node_limit - node_used)
        {
            return Err(format!("Capacity exhausted: requested {items} items and {bytes} bytes; retained {} items and {} bytes{}.", inner.used.items, inner.used.bytes, node.map(|id| format!("; node {id:?} retains {node_used} bytes")).unwrap_or_default()));
        }
        inner.used.items += items;
        inner.used.bytes += bytes;
        if let Some(node) = node {
            *inner.nodes.entry(node.into()).or_default() += bytes;
        }
        Ok(Reservation {
            budget: self.clone(),
            items,
            bytes,
            node: node.map(String::from),
        })
    }
    /// Retain a parser, collector, tool result or controller value under its node's budget.
    /// A replacement must be admitted while the old value remains charged, then swapped.
    pub fn retain<T: Serialize>(&self, node: &str, value: T) -> Result<Retained<T>, String> {
        let limit = self.0.lock().unwrap().node_limit;
        let bytes = crate::pump::measure(&value, limit).map_err(|e| e.to_string())?;
        let reservation = self.reserve(0, bytes, Some(node))?;
        Ok(Retained::charged(value, vec![reservation]))
    }
}
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: RetentionBudget,
    items: usize,
    bytes: usize,
    node: Option<String>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut inner = self.budget.0.lock().unwrap();
        inner.used.items -= self.items;
        inner.used.bytes -= self.bytes;
        if let Some(node) = &self.node {
            if let Some(used) = inner.nodes.get_mut(node) {
                *used -= self.bytes;
                if *used == 0 {
                    inner.nodes.remove(node);
                }
            }
        }
    }
}
#[derive(Debug)]
struct RetainedInner<T> {
    value: T,
    _reservations: Vec<Reservation>,
}
/// Clones share the immutable payload and keep its reservation until the last owner drops.
#[derive(Debug)]
pub struct Retained<T>(Arc<RetainedInner<T>>);
impl<T> Clone for Retained<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Retained<T> {
    pub(crate) fn charged(value: T, reservations: Vec<Reservation>) -> Self {
        Self(Arc::new(RetainedInner {
            value,
            _reservations: reservations,
        }))
    }
}
impl<T> Deref for Retained<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0.value
    }
}
impl<T> AsRef<T> for Retained<T> {
    fn as_ref(&self) -> &T {
        &self.0.value
    }
}
impl<T> std::borrow::Borrow<T> for Retained<T> {
    fn borrow(&self) -> &T {
        &self.0.value
    }
}
impl<T: Serialize> Serialize for Retained<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.value.serialize(serializer)
    }
}
