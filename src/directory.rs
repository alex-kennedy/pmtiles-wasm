use integer_encoding::VarIntReader;
use std::io::Cursor;
use std::u64;

use crate::error::{PMTilesError, PMTilesResult};
use crate::tile_id::TileID;

/// A directory entry describes either where a specific tile can be found in the tile data section
/// or where a leaf directory can be found in the leaf directories section.
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DirectoryEntry {
    /// Specifies the ID of the tile or the first tile in the leaf directory.
    pub tile_id: TileID,

    /// Specifies the offset of the first byte of the tile or leaf directory. This address offset
    /// is relative to the first byte of the tile data section for tile entries and relative to the
    /// first byte of the leaf directories section for leaf directory entries.
    pub offset: u64,

    /// Specifies the number of bytes of this tile or leaf directory. This size always indicates
    /// the compressed size, if the tile or leaf directory is compressed. The length MUST be
    /// greater than 0.
    pub length: u64,

    /// Specifies the number of tiles for which this entry is valid. A run length of 0 means that
    /// this entry is for a leaf directory and not for a tile.
    pub run_length: u64,
}

impl DirectoryEntry {
    /// When true, this directory entry points to a leaf directory in the leaf directories section.
    /// Otherwise, this directory entry points to a tile in the tile data section.
    pub fn is_leaf_directory(&self) -> bool {
        self.run_length == 0
    }
}

impl Directory {
    /// Returns the directory entry for the given tile ID, or None if there is no matching entry.
    pub(crate) fn get_entry(&self, tile_id: TileID) -> Option<&DirectoryEntry> {
        let i = self.entries.partition_point(|e| e.tile_id <= tile_id);
        if i == 0 {
            return None;
        }
        let entry = &self.entries[i - 1];
        if entry.is_leaf_directory() || tile_id.id() - entry.tile_id.id() < entry.run_length {
            return Some(entry);
        }
        None
    }
}

/// A directory is simply a list of entries. Each entry describes either where a specific tile can
/// be found in the tile data section or where a leaf directory can be found in the leaf directories
/// section.
pub struct Directory {
    entries: Vec<DirectoryEntry>,
}

impl TryFrom<&[u8]> for Directory {
    type Error = PMTilesError;

    /// An encoded directory consists of five parts in the following order:
    ///  1. The number of entries contained in the directory (MUST be greater than 0)
    ///  2. TileIDs of all entries
    ///  3. RunLengths of all entries
    ///  4. Lengths of all entries
    ///  5. Offsets of all entries
    fn try_from(bytes: &[u8]) -> PMTilesResult<Self> {
        let mut cursor = Cursor::new(bytes);

        // 1. The number of entries contained in the directory (MUST be greater than 0)
        let n_entries = cursor
            .read_varint::<usize>()
            .map_err(|e| PMTilesError::DirectoryDecodeError(e.to_string()))?;
        if n_entries == 0 {
            return Err(PMTilesError::DirectoryDecodeError(
                "number of directory entries == 0, must be > 0".to_string(),
            ));
        }
        if n_entries > bytes.len() / 4 {
            return Err(PMTilesError::DirectoryDecodeError(format!(
                "{} directory entries cannot fit in {} bytes",
                n_entries,
                bytes.len()
            )));
        }

        let mut entries = vec![DirectoryEntry::default(); n_entries as usize];

        // 2. TileIDs of all entries. The TileIDs are delta-encoded, i.e., the number to be written
        // is the difference to the last TileID.
        let mut last_tile_id: u64 = 0;
        for i in 0..n_entries {
            let tile_id_difference = cursor
                .read_varint::<u64>()
                .map_err(|e| PMTilesError::DirectoryDecodeError(e.to_string()))?;
            last_tile_id = match last_tile_id.checked_add(tile_id_difference) {
                Some(v) => v,
                None => {
                    return Err(PMTilesError::DirectoryDecodeError(format!(
                        "tile ID decoding overflow, last_tile_id={}, tile_id_difference={}",
                        last_tile_id, tile_id_difference
                    )));
                }
            };
            entries[i].tile_id = last_tile_id.try_into()?;
        }

        // 3. RunLengths of all entries
        for i in 0..n_entries {
            let run_length = cursor
                .read_varint::<u64>()
                .map_err(|e| PMTilesError::DirectoryDecodeError(e.to_string()))?;
            entries[i].run_length = run_length;
        }

        // 4. Lengths of all entries
        for i in 0..n_entries {
            let length = cursor
                .read_varint::<u64>()
                .map_err(|e| PMTilesError::DirectoryDecodeError(e.to_string()))?;
            if length == 0 {
                return Err(PMTilesError::DirectoryDecodeError(
                    "directory entry length == 0, must be > 0".to_string(),
                ));
            }
            entries[i].length = length;
        }

        // 5. Offsets of all entries
        for i in 0..n_entries {
            // Offsets are encoded either as `Offset + 1` or `0`, if they are equal to the sum of
            // the offset and length of the previous entry (tile blobs are contiguous).
            let offset_encoded = cursor
                .read_varint::<u64>()
                .map_err(|e| PMTilesError::DirectoryDecodeError(e.to_string()))?;
            if offset_encoded > 0 {
                entries[i].offset = offset_encoded - 1;
            } else if i >= 1 {
                entries[i].offset = entries[i - 1]
                    .offset
                    .checked_add(entries[i - 1].length)
                    .ok_or_else(|| {
                        PMTilesError::DirectoryDecodeError(format!(
                            "directory entry {} offset overflows",
                            i
                        ))
                    })?;
            } else {
                return Err(PMTilesError::DirectoryDecodeError(
                    "first directory entry offset == 0, must be > 0".to_string(),
                ));
            }
        }

        Ok(Directory { entries })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn entry(tile_id: u64, offset: u64, length: u64, run_length: u64) -> DirectoryEntry {
        DirectoryEntry {
            tile_id: TileID::new(tile_id).unwrap(),
            offset,
            length,
            run_length,
        }
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_varint_cursor() {
        let b: Vec<u8> = vec![0, 1, 127, 0xe5, 0x8e, 0x26];
        let mut cursor = Cursor::new(b);

        assert_eq!(cursor.read_varint::<usize>().unwrap(), 0);
        assert_eq!(cursor.read_varint::<usize>().unwrap(), 1);
        assert_eq!(cursor.read_varint::<usize>().unwrap(), 127);
        assert_eq!(cursor.read_varint::<usize>().unwrap(), 624485);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_large_varint() {
        let b: Vec<u8> = vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x0f];

        assert_eq!(
            Cursor::new(b).read_varint::<u64>().unwrap(),
            9007199254740991
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory() {
        let bytes = [3, 0, 1, 1, 1, 1, 2, 1, 1, 2, 1, 0, 6];

        let directory = Directory::try_from(bytes.as_slice()).unwrap();

        assert_eq!(
            directory.entries,
            vec![entry(0, 0, 1, 1), entry(1, 1, 1, 1), entry(2, 5, 2, 2)]
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_with_leaf() {
        let bytes = [2, 0, 100, 1, 0, 10, 20, 1, 0];

        let directory = Directory::try_from(bytes.as_slice()).unwrap();

        assert_eq!(
            directory.entries,
            vec![entry(0, 0, 10, 1), entry(100, 10, 20, 0)]
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_no_entries() {
        let bytes = [0];

        let result = Directory::try_from(bytes.as_slice());

        assert!(matches!(result, Err(PMTilesError::DirectoryDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_zero_length() {
        let bytes = [1, 0, 1, 0, 1];

        let result = Directory::try_from(bytes.as_slice());

        assert!(matches!(result, Err(PMTilesError::DirectoryDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_first_offset_contiguous() {
        let bytes = [1, 0, 1, 1, 0];

        let result = Directory::try_from(bytes.as_slice());

        assert!(matches!(result, Err(PMTilesError::DirectoryDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_truncated() {
        let bytes = [2, 0, 1, 1];

        let result = Directory::try_from(bytes.as_slice());

        assert!(matches!(result, Err(PMTilesError::DirectoryDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_too_many_entries() {
        let bytes = [0xff, 0xff, 0xff, 0xff, 0x0f, 0, 1, 1, 1];

        let result = Directory::try_from(bytes.as_slice());

        assert!(matches!(result, Err(PMTilesError::DirectoryDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_directory_contiguous_offset_overflow() {
        let mut bytes = vec![2, 0, 1, 1, 1];
        bytes.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        bytes.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        bytes.extend_from_slice(&[2, 0]);

        let result = Directory::try_from(bytes.as_slice());

        assert!(matches!(result, Err(PMTilesError::DirectoryDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_huge_run_length() {
        let directory = Directory {
            entries: vec![entry(100, 1, 1, u64::MAX)],
        };

        let got = directory.get_entry(TileID::new(150).unwrap());

        assert_eq!(got, Some(&entry(100, 1, 1, u64::MAX)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_missing() {
        let directory = Directory { entries: vec![] };

        let got = directory.get_entry(TileID::new(101).unwrap());

        assert_eq!(got, None);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_first_entry_equals_id() {
        let directory = Directory {
            entries: vec![entry(100, 1, 1, 1)],
        };

        let got = directory.get_entry(TileID::new(100).unwrap());
        let next = directory.get_entry(TileID::new(101).unwrap());

        assert_eq!(got, Some(&entry(100, 1, 1, 1)));
        assert_eq!(next, None);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_before_first_entry() {
        let directory = Directory {
            entries: vec![entry(100, 1, 1, 1)],
        };

        let got = directory.get_entry(TileID::new(99).unwrap());

        assert_eq!(got, None);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_with_run_length() {
        let directory = Directory {
            entries: vec![entry(3, 3, 1, 2), entry(5, 5, 1, 2)],
        };

        let got = directory.get_entry(TileID::new(4).unwrap());

        assert_eq!(got, Some(&entry(3, 3, 1, 2)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_single_entry_run_length() {
        let directory = Directory {
            entries: vec![entry(100, 1, 1, 2)],
        };

        let got = directory.get_entry(TileID::new(101).unwrap());

        assert_eq!(got, Some(&entry(100, 1, 1, 2)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_last_entry_run_length() {
        let directory = Directory {
            entries: vec![entry(100, 1, 1, 1), entry(150, 2, 2, 2)],
        };

        let got = directory.get_entry(TileID::new(151).unwrap());

        assert_eq!(got, Some(&entry(150, 2, 2, 2)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_first_of_many_run_length() {
        let directory = Directory {
            entries: vec![entry(50, 1, 1, 2), entry(100, 2, 2, 1), entry(150, 3, 3, 1)],
        };

        let got = directory.get_entry(TileID::new(51).unwrap());

        assert_eq!(got, Some(&entry(50, 1, 1, 2)));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_get_entry_leaf() {
        let directory = Directory {
            entries: vec![entry(100, 1, 1, 0)],
        };

        let got = directory.get_entry(TileID::new(150).unwrap());

        assert_eq!(got, Some(&entry(100, 1, 1, 0)));
    }
}
