use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use anyhow::{Context, Result};
use brain_domain::SourceCursor;
use sha2::{Digest, Sha256};

use crate::traits::{
    FileIdentity, FileRotation, RawRecord, RawRecordBatch, ReadOutcome, SourceDescriptor,
};

pub(crate) fn read_jsonl_increment(
    source: &SourceDescriptor,
    cursor: &SourceCursor,
) -> Result<ReadOutcome> {
    let metadata = std::fs::metadata(&source.path)
        .with_context(|| format!("failed to inspect source {}", source.path.display()))?;
    let current_identity = file_identity(&source.path)?;
    let rotated = cursor
        .file_identity
        .as_ref()
        .is_some_and(|previous| previous != &current_identity.0)
        || metadata.len() < cursor.byte_offset;
    let start_offset = if rotated { 0 } else { cursor.byte_offset };
    let rotation = rotated.then(|| FileRotation {
        previous_identity: cursor.file_identity.clone(),
        current_identity: current_identity.0.clone(),
        previous_offset: cursor.byte_offset,
        current_size: metadata.len(),
    });

    let file = File::open(&source.path)
        .with_context(|| format!("failed to open source {}", source.path.display()))?;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(start_offset))?;
    let mut records = Vec::new();
    let mut committed_offset = start_offset;

    loop {
        let record_offset = committed_offset;
        let mut bytes = Vec::new();
        let bytes_read = reader.read_until(b'\n', &mut bytes)?;
        if bytes_read == 0 {
            break;
        }
        if bytes.last() != Some(&b'\n') {
            break;
        }
        committed_offset += bytes_read as u64;
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        let raw_text = String::from_utf8_lossy(&bytes).into_owned();
        let parsed = serde_json::from_slice::<serde_json::Value>(&bytes);
        let (value, parse_error) = match parsed {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let raw_hash: [u8; 32] = Sha256::digest(&bytes).into();
        records.push(RawRecord {
            source_id: source.source_id.clone(),
            source_locator: source.path.to_string_lossy().into_owned(),
            byte_offset: record_offset,
            next_byte_offset: committed_offset,
            value,
            raw_text,
            parse_error,
            raw_hash,
        });
    }

    if records.is_empty() && rotation.is_none() {
        return Ok(ReadOutcome::NoChange);
    }
    Ok(ReadOutcome::Batch(RawRecordBatch {
        records,
        next_cursor: SourceCursor::for_file(committed_offset, current_identity.0.clone()),
        file_identity: current_identity,
        last_complete_newline: committed_offset,
        rotation,
    }))
}

pub(crate) fn file_identity(path: &std::path::Path) -> Result<FileIdentity> {
    let identity = file_id::get_file_id(path)?;
    let value = match identity {
        file_id::FileId::Inode {
            device_id,
            inode_number,
        } => format!("inode:{device_id:016x}:{inode_number:016x}"),
        file_id::FileId::LowRes {
            volume_serial_number,
            file_index,
        } => format!("windows-low:{volume_serial_number:08x}:{file_index:016x}"),
        file_id::FileId::HighRes {
            volume_serial_number,
            file_id,
        } => format!("windows-high:{volume_serial_number:016x}:{file_id:032x}"),
    };
    Ok(FileIdentity(value))
}
