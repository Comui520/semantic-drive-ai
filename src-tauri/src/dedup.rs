use crate::scanner::FileEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub id: String,
    pub files: Vec<DuplicateFile>,
    pub total_wasted: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: String,
}

/// Find exact duplicates by comparing file hashes and sizes.
pub fn find_duplicates(files: &[FileEntry], device_root: &Path) -> Vec<DuplicateGroup> {
    let mut size_map: HashMap<u64, Vec<&FileEntry>> = HashMap::new();

    // First pass: group by size (quick filter)
    for file in files {
        if file.size == 0 {
            continue;
        }
        size_map.entry(file.size).or_default().push(file);
    }

    let mut hash_map: HashMap<String, Vec<&FileEntry>> = HashMap::new();

    // Second pass: compute hashes for files with same size
    for (_size, group) in size_map.iter() {
        if group.len() < 2 {
            continue;
        }
        for file in group {
            let full_path = device_root.join(&file.path);
            if let Some(hash) = crate::scanner::compute_hash(&full_path) {
                hash_map.entry(hash).or_default().push(file);
            }
        }
    }

    // Build duplicate groups (hash must have 2+ files)
    let mut groups: Vec<DuplicateGroup> = hash_map
        .into_iter()
        .filter(|(_, files)| files.len() >= 2)
        .map(|(hash, files)| {
            let dup_files: Vec<DuplicateFile> = files
                .iter()
                .map(|f| DuplicateFile {
                    name: f.name.clone(),
                    path: f.path.clone(),
                    size: f.size,
                    modified: f.modified.clone(),
                })
                .collect();

            let total_wasted = if !dup_files.is_empty() {
                dup_files[0].size * (dup_files.len() as u64 - 1)
            } else {
                0
            };

            DuplicateGroup {
                id: hash[..12].to_string(),
                files: dup_files,
                total_wasted,
                reason: "文件内容完全相同".to_string(),
            }
        })
        .collect();

    // Sort by wasted space (largest first)
    groups.sort_by(|a, b| b.total_wasted.cmp(&a.total_wasted));

    groups
}
