//! Workflow-independent publication of general groups and date/group folder labels.
use super::{Features, Input, cluster};
use crate::core::{model::MediaId, subjects::store::Store};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
#[derive(Clone)]
pub struct Record {
    pub id: MediaId,
    pub key: Option<String>,
    pub portrait: bool,
    pub date: Option<String>,
}
pub struct Output {
    pub groups: HashMap<MediaId, usize>,
    pub folders: HashMap<MediaId, String>,
}
pub fn group(
    records: &[Record],
    store: &Store,
    mixed: bool,
    eps: f32,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Output>> {
    let mut buckets = BTreeMap::<Option<&str>, Vec<&Record>>::new();
    for record in records {
        if cancelled() {
            return Ok(None);
        }
        buckets
            .entry(if mixed {
                record.date.as_deref()
            } else {
                Some("")
            })
            .or_default()
            .push(record);
    }
    let mut output = Output {
        groups: HashMap::new(),
        folders: HashMap::new(),
    };
    let mut next = 0;
    for (date, bucket) in buckets {
        if cancelled() {
            return Ok(None);
        }
        if mixed && date.is_none() {
            for record in bucket {
                output.folders.insert(record.id, "undated".into());
            }
            continue;
        }
        let members: Vec<_> = bucket.iter().filter(|r| r.key.is_some()).collect();
        let inputs: Vec<_> = members
            .iter()
            .map(|r| Input {
                key: r.key.clone().unwrap(),
                portrait: r.portrait,
            })
            .collect();
        let Some(result) = cluster(
            &inputs,
            eps,
            |key| {
                Features::new(
                    store
                        .regional(key)
                        .context("Regional evidence is no longer available; retry analysis")?
                        .vectors,
                )
            },
            &cancelled,
        )?
        else {
            return Ok(None);
        };
        let mut groups = BTreeMap::<usize, Vec<MediaId>>::new();
        for (record, label) in members.into_iter().zip(result.labels) {
            groups.entry(label).or_default().push(record.id);
        }
        let mut local = 0;
        for members in groups.values() {
            next += 1;
            let folder = if mixed {
                if members.len() > 1 {
                    local += 1;
                    format!("{}/group-{local}", date.unwrap())
                } else {
                    date.unwrap().into()
                }
            } else {
                String::new()
            };
            for &id in members {
                if cancelled() {
                    return Ok(None);
                }
                output.groups.insert(id, next);
                if mixed {
                    output.folders.insert(id, folder.clone());
                }
            }
        }
        if mixed {
            for record in bucket.into_iter().filter(|r| r.key.is_none()) {
                output.folders.insert(record.id, date.unwrap().into());
            }
        }
    }
    Ok((!cancelled()).then_some(output))
}
