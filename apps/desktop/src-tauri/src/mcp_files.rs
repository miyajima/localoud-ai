use super::{err, safe_path, Arguments, Result, FILE_BYTES};
use cap_std::{
    ambient_authority,
    fs::{Dir, MetadataExt, OpenOptions, OpenOptionsExt},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

const INPUT_BYTES: usize = 32 * 1024;

pub(super) fn edit_text(
    current: &str,
    expected_sha256: &str,
    old_text: &str,
    new_text: &str,
) -> Result<String> {
    if expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("expected_sha256 must be the lowercase SHA-256 from file_read".into());
    }
    if format!("{:x}", Sha256::digest(current.as_bytes())) != expected_sha256 {
        return Err("File changed. Read it again before editing (sha256 mismatch).".into());
    }
    if old_text.is_empty()
        || old_text == new_text
        || old_text.len() + new_text.len() > INPUT_BYTES
        || new_text.contains('\0')
        || old_text.contains('\0')
    {
        return Err(
            "Use a nonempty old_text and different new_text, without NUL, within 32KiB combined."
                .into(),
        );
    }
    let start = current
        .find(old_text)
        .ok_or("old_text was not found; read the file again")?;
    // Overlapping matches are also ambiguous (e.g. 'aa' in 'aaa').
    if current[start + current[start..].chars().next().unwrap().len_utf8()..].contains(old_text) {
        return Err("old_text must occur exactly once; provide more context".into());
    }
    if current.len() - old_text.len() + new_text.len() > FILE_BYTES as usize {
        return Err("Edited file exceeds the 1MiB text limit".into());
    }
    Ok(format!(
        "{}{}{}",
        &current[..start],
        new_text,
        &current[start + old_text.len()..]
    ))
}
pub(super) fn mutate_file(root: &Path, tool: &str, args: &Arguments) -> Result<Value> {
    let path = args.path.as_deref().ok_or("path is required")?;
    if path.len() > 4096
        || !safe_path(path)
        || Path::new(path).components().any(|c| {
            matches!(
                c.as_os_str()
                    .to_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str(),
                "agents.md" | "claude.md" | "skill.md"
            )
        })
    {
        return Err("path is protected or outside the project".into());
    }
    if args.query.is_some()
        || args.base_revision.is_some()
        || args.start_line.is_some()
        || args.line_count.is_some()
        || args.offset.is_some()
        || args.limit.is_some()
        || args.log_index.is_some()
    {
        return Err("Unexpected arguments for file mutation".into());
    }
    if root.canonicalize().map_err(err)? != root {
        return Err("Project root moved or became a symlink".into());
    }
    // Each directory is opened relative to its capability with O_NOFOLLOW. No
    // user-supplied absolute path or symlink is used for a filesystem write.
    let mut dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(err)?;
    let mut parts = path.split('/').peekable();
    let mut leaf = "";
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            leaf = part;
            break;
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
        dir = Dir::from_std_file(dir.open_with(part, &options).map_err(err)?.into_std());
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    let (text, previous, permissions) = if tool == "file_create" {
        if args.expected_sha256.is_some() || args.old_text.is_some() || args.new_text.is_some() {
            return Err("file_create accepts only project_id, path and content".into());
        }
        let text = args.content.as_deref().ok_or("content is required")?;
        if text.len() > INPUT_BYTES || text.contains('\0') {
            return Err("content must be UTF-8 without NUL and at most 32KiB".into());
        }
        (text.to_owned(), None, None)
    } else if tool == "file_edit" {
        if args.content.is_some() {
            return Err("file_edit uses old_text/new_text, not content".into());
        }
        let expected = args
            .expected_sha256
            .as_deref()
            .ok_or("expected_sha256 is required; read the file first")?;
        let old = args.old_text.as_deref().ok_or("old_text is required")?;
        let new = args.new_text.as_deref().ok_or("new_text is required")?;
        let current = EditableFile::read(&dir, leaf)?;
        // Redacted files cannot be reconstructed safely through this interface.
        if hub_policy::redact(&current.text) != current.text {
            return Err(
                "Files containing redacted sensitive content cannot be edited via MCP".into(),
            );
        }
        (
            edit_text(&current.text, expected, old, new)?,
            Some(expected.to_owned()),
            Some(current.permissions),
        )
    } else {
        return Err("Unknown mutation tool".into());
    };
    if hub_policy::redact(&text) != text {
        return Err("Sensitive content cannot be written via MCP".into());
    }
    let temp = format!(".localoud-edit-{operation_id}");
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = dir.open_with(&temp, &options).map_err(err)?;
        file.write_all(text.as_bytes()).map_err(err)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions).map_err(err)?;
        }
        file.sync_all().map_err(err)?;
        if let Some(expected) = &previous {
            // Recheck immediately before replacement. The shared server lock
            // guarantees only one MCP edit can commit for a given source hash.
            let live = EditableFile::read(&dir, leaf)?;
            if format!("{:x}", Sha256::digest(live.text.as_bytes())) != *expected {
                return Err("File changed during editing; read it again".into());
            }
            dir.rename(&temp, &dir, leaf).map_err(err)?;
        } else {
            // hard_link publishes the completed file atomically and fails if
            // the destination exists (including dangling symlinks).
            dir.hard_link(&temp, &dir, leaf).map_err(err)?;
        }
        Ok(())
    })();
    let _ = dir.remove_file(&temp);
    result?;
    Ok(
        json!({"operation_id":operation_id,"project_id":args.project_id,"path":path,"operation":tool,"previous_sha256":previous,"sha256":format!("{:x}",Sha256::digest(text.as_bytes())),"bytes":text.len()}),
    )
}

struct EditableFile {
    text: String,
    permissions: cap_std::fs::Permissions,
}
impl EditableFile {
    fn read(dir: &Dir, leaf: &str) -> Result<Self> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = dir.open_with(leaf, &options).map_err(err)?;
        let meta = file.metadata().map_err(err)?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.len() > FILE_BYTES
            || meta.permissions().readonly()
        {
            return Err(
                "Edit requires a writable regular text file, without hard links, within 1MiB"
                    .into(),
            );
        }
        let mut text = String::new();
        file.take(FILE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|_| "File is not UTF-8 text")?;
        if text.len() as u64 > FILE_BYTES || text.contains('\0') {
            return Err("File is not bounded text".into());
        }
        Ok(Self {
            text,
            permissions: meta.permissions(),
        })
    }
}
