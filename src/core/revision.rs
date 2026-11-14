//! Change stamps: a cheap way for a view to know that a value it worked
//! from has changed, without comparing the value itself.
//!
//! A stamp is unique in the process, so two values with the same stamp are
//! the same value, wherever they came from: one replaced by another never
//! looks unchanged.
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A stamp no value has had before.
pub fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A value with the stamp of its last change. Reading it leaves the stamp
/// alone; any mutable access takes a new one, whether or not it ends up
/// changing the value, so a change can never go unnoticed.
#[derive(Debug)]
pub struct Revised<T> {
    value: T,
    revision: u64,
}

impl<T> Revised<T> {
    /// The stamp of the last change.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

impl<T> From<T> for Revised<T> {
    fn from(value: T) -> Self {
        Self {
            value,
            revision: next(),
        }
    }
}

impl<T: Default> Default for Revised<T> {
    fn default() -> Self {
        T::default().into()
    }
}

impl<T> Deref for Revised<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Revised<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.revision = next();
        &mut self.value
    }
}

impl<T: PartialEq> PartialEq<T> for Revised<T> {
    fn eq(&self, other: &T) -> bool {
        self.value == *other
    }
}

impl<'a, T> IntoIterator for &'a Revised<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.value.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mutable_access_and_replacement_take_a_new_stamp() {
        let mut list: Revised<Vec<u8>> = vec![1, 2].into();
        let first = list.revision();
        // Reading, comparing and iterating leave it alone.
        assert_eq!(list.len(), 2);
        assert!(list == vec![1, 2]);
        assert_eq!((&list).into_iter().count(), 2);
        assert_eq!(list.revision(), first);
        list.push(3);
        let second = list.revision();
        assert_ne!(second, first);
        // A value put in its place is never taken for the one before.
        list = vec![1, 2, 3].into();
        assert!(list.revision() != second && list.revision() != first);
    }
}
