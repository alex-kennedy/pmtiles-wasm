use crate::Compression;
use crate::{PMTilesError, PMTilesResult};

use wasm_bindgen::prelude::*;

/// The magic number is a fixed 7-byte field whose value is always PMTiles in UTF-8 encoding.
const MAGIC_NUMBER: &[u8] = "PMTiles".as_bytes();

/// The version is a fixed 1-byte field whose value is always 3 (0x03).
const VERSION: u8 = 3;

#[allow(unused)]
#[derive(Debug, Clone, Copy)]
#[wasm_bindgen]
pub struct Header {
    /// The Root Directory Offset is an 8-byte field whose value gives the offset of the first byte
    /// of the root directory. This address offset is relative to the first byte of the archive.
    pub(crate) root_directory_offset: u64,

    /// The Root Directory Length is an 8-byte field specifying the number of bytes in the root
    /// directory.
    pub(crate) root_directory_length: u64,

    /// The Metadata Offset is an 8-byte field whose value gives the offset of the first byte of the
    /// metadata. This address offset is relative to the first byte of the archive.
    pub(crate) metadata_offset: u64,

    /// The Metadata Length is an 8-byte field specifying the number of bytes of metadata.
    pub(crate) metadata_length: u64,

    /// The Leaf Directories Offset is an 8-byte field whose value gives the offset of the first
    /// byte of the leaf directories. This address offset is relative to the first byte of the
    /// archive.
    pub(crate) leaf_directories_offset: u64,

    /// The Leaf Directories Length is an 8-byte field specifying the accumulated size (in bytes)
    /// of all leaf directories. A value of 0 indicates that there are no leaf directories included
    /// in this PMTiles archive.
    pub(crate) leaf_directories_length: u64,

    /// The Tile Data Offset is an 8-byte field whose value gives the offset of the first byte of
    /// the tile data. This address offset is relative to the first byte of the archive.
    pub(crate) tile_data_offset: u64,

    /// The Tile Data Length is an 8-byte field specifying the accumulated size (in bytes) of all
    /// tiles in the tile data section.
    pub(crate) tile_data_length: u64,

    /// The Number of Addressed Tiles is an 8-byte field specifying the total number of tiles in the
    /// PMTiles archive, before RunLength Encoding. A value of 0 indicates that the number is
    /// unknown.
    pub number_of_addressed_tiles: u64,

    /// The Number of Tile Entries is an 8-byte field specifying the total number of tile entries:
    /// entries where RunLength is greater than 0.
    pub number_of_tile_entries: u64,

    /// The Number of Tile Contents is an 8-byte field specifying the total number of blobs in the
    /// tile data section.
    pub number_of_tile_contents: u64,

    /// Clustered is a 1-byte field specifying if the data of the individual tiles in the data
    /// section is ordered by their TileID (clustered) or not (not clustered).
    pub clustered: Clustered,

    /// The Internal Compression is a 1-byte field specifying the compression of the root directory,
    /// metadata, and all leaf directories.
    pub internal_compression: Compression,

    /// The Tile Compression is a 1-byte field specifying the compression of all tiles.
    pub tile_compression: Compression,

    /// The Tile Type is a 1-byte field specifying the type of tiles.
    pub tile_type: TileType,

    /// The Min Zoom is a 1-byte field specifying the minimum zoom of the tiles.
    pub min_zoom: u8,

    /// The Max Zoom is a 1-byte field specifying the maximum zoom of the tiles. It must be greater
    /// than or equal to the min zoom.
    pub max_zoom: u8,

    /// The Min Position is an 8-byte field that includes the minimum latitude and minimum longitude
    /// of the bounds.
    pub min_position: LatLng,

    /// The Max Position is an 8-byte field including the maximum latitude and maximum longitude of
    /// the bounds.
    pub max_position: LatLng,

    /// The Center Zoom is a 1-byte field specifying the center zoom (LOD) of the tiles. A reader
    /// MAY use this as the initial zoom when displaying tiles from the PMTiles archive.
    pub center_zoom: u8,

    /// The Center Position is an 8-byte field that includes the latitude and longitude of the
    /// center position. A reader MAY use this as the initial center position when displaying tiles
    /// from the PMTiles archive.
    pub center_position: LatLng,
}

impl Header {
    /// Decode a 127-byte header, per the PMTiles V3 spec.
    ///
    /// See <http://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md#31-overview>.
    ///
    /// ```text
    /// Offset     00   01   02   03   04   05   06   07   08   09   0A   0B   0C   0D   0E   0F
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000000   |           Magic Number           |  V |         Root Directory Offset         |
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000010   |         Root Directory Length         |            Metadata Offset            |
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000020   |            Metadata Length            |        Leaf Directories Offset        |
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000030   |        Leaf Directories Length        |            Tile Data Offset           |
    ///         +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000040   |            Tile Data Length           |         Num of Addressed Tiles        |
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000050   |         Number of Tile Entries        |        Number of Tile Contents        |
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000060   |  C | IC | TC | TT |MinZ|MaxZ|              Min Position             |      Max
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// 000070    Position                     |CenZ|            Center Position            |
    ///          +----+----+----+----+----+----+----+----+----+----+----+----+----+----+----+
    /// ```
    pub fn decode(bytes: &[u8; 127]) -> PMTilesResult<Header> {
        if &bytes[0..7] != MAGIC_NUMBER {
            return Err(PMTilesError::HeaderDecodeError(format!(
                "invalid magic number {:#?}, expected 'PMTiles'",
                &bytes[0..7]
            )));
        }
        if bytes[7] != VERSION {
            return Err(PMTilesError::HeaderDecodeError(format!(
                "invalid version {}, expected 3",
                bytes[7]
            )));
        }
        Ok(Header {
            root_directory_offset: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            root_directory_length: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            metadata_offset: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            metadata_length: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            leaf_directories_offset: u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            leaf_directories_length: u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            tile_data_offset: u64::from_le_bytes(bytes[56..64].try_into().unwrap()),
            tile_data_length: u64::from_le_bytes(bytes[64..72].try_into().unwrap()),
            number_of_addressed_tiles: u64::from_le_bytes(bytes[72..80].try_into().unwrap()),
            number_of_tile_entries: u64::from_le_bytes(bytes[80..88].try_into().unwrap()),
            number_of_tile_contents: u64::from_le_bytes(bytes[88..96].try_into().unwrap()),
            clustered: Clustered::try_from(bytes[96])?,
            internal_compression: Compression::try_from(bytes[97])?,
            tile_compression: Compression::try_from(bytes[98])?,
            tile_type: TileType::try_from(bytes[99])?,
            min_zoom: bytes[100],
            max_zoom: bytes[101],
            min_position: LatLng::decode(&bytes[102..110].try_into().unwrap()),
            max_position: LatLng::decode(&bytes[110..118].try_into().unwrap()),
            center_zoom: bytes[118],
            center_position: LatLng::decode(&bytes[119..127].try_into().unwrap()),
        })
    }
}

/// Clustered is a 1-byte field specifying if the data of the individual tiles in the data section
/// is ordered by their TileID (clustered) or not (not clustered). Therefore, Clustered means that:
///   - offsets are either contiguous with the previous offset+length, or refer to a lesser offset
///     when writing with deduplication.
///   - the first tile entry in the directory has offset 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[wasm_bindgen]
#[non_exhaustive]
pub enum Clustered {
    NotClustered,
    Clustered,
}

impl TryFrom<u8> for Clustered {
    type Error = PMTilesError;

    fn try_from(value: u8) -> PMTilesResult<Self> {
        match value {
            0 => Ok(Clustered::NotClustered),
            1 => Ok(Clustered::Clustered),
            _ => Err(PMTilesError::HeaderDecodeError(format!(
                "invalid clustered value {}",
                value
            ))),
        }
    }
}

#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TileType {
    /// Unknown / Other
    Unknown,
    /// MVT Vector Tile
    Mvt,
    /// PNG
    Png,
    /// JPEG
    Jpeg,
    /// WebP
    WebP,
    /// AVIF
    Avif,
    /// MapLibre Vector Tile
    MapLibreVectorTile,
}

impl TryFrom<u8> for TileType {
    type Error = PMTilesError;

    // See <https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md#tile-type-tt>.
    fn try_from(value: u8) -> PMTilesResult<Self> {
        match value {
            0 => Ok(TileType::Unknown),
            1 => Ok(TileType::Mvt),
            2 => Ok(TileType::Png),
            3 => Ok(TileType::Jpeg),
            4 => Ok(TileType::WebP),
            5 => Ok(TileType::Avif),
            6 => Ok(TileType::MapLibreVectorTile),
            _ => Err(PMTilesError::HeaderDecodeError(format!(
                "invalid tile type {}",
                value
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[wasm_bindgen]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

impl LatLng {
    fn decode(bytes: &[u8; 8]) -> Self {
        LatLng {
            lat: decode_lat_or_lng(bytes[4..8].try_into().unwrap()),
            lng: decode_lat_or_lng(bytes[0..4].try_into().unwrap()),
        }
    }
}

fn decode_lat_or_lng(bytes: &[u8; 4]) -> f64 {
    let l = i32::from_le_bytes(*bytes) as f64;
    l / 10_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    use approx::assert_relative_eq;
    use rstest::rstest;

    #[rstest]
    #[test_attr(wasm_bindgen_test(unsupported = test))]
    #[case([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0.0, 0.0)]
    #[case([128, 240, 250, 2, 128, 195, 201, 1], 3.0, 5.0)]
    #[case([9, 50, 20, 210, 141, 222, 209, 248], -12.0463731, -77.0428407)]
    #[case([0, 14, 39, 7, 128, 139, 45, 229], -45.0, 12.0)]
    // i32::MIN / i32::MAX boundaries.
    #[case([0, 0, 0, 128, 255, 255, 255, 127], 214.7483647, -214.7483648)]
    fn test_lat_lng_decode(#[case] b: [u8; 8], #[case] lat: f64, #[case] lng: f64) {
        let lat_lng = LatLng::decode(&b);

        assert_relative_eq!(lat_lng.lat, lat);
        assert_relative_eq!(lat_lng.lng, lng);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_header() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PMTiles");
        bytes.push(3);
        for value in 1u64..=11 {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[1, 1, 2, 3, 12, 13]);
        for value in [14i32, 15, 16, 17] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(14);
        for value in [18i32, 19] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        let header = Header::decode(bytes.as_slice().try_into().unwrap()).unwrap();

        assert_eq!(header.root_directory_offset, 1);
        assert_eq!(header.root_directory_length, 2);
        assert_eq!(header.metadata_offset, 3);
        assert_eq!(header.metadata_length, 4);
        assert_eq!(header.leaf_directories_offset, 5);
        assert_eq!(header.leaf_directories_length, 6);
        assert_eq!(header.tile_data_offset, 7);
        assert_eq!(header.tile_data_length, 8);
        assert_eq!(header.number_of_addressed_tiles, 9);
        assert_eq!(header.number_of_tile_entries, 10);
        assert_eq!(header.number_of_tile_contents, 11);
        assert_eq!(header.clustered, Clustered::Clustered);
        assert_eq!(header.internal_compression, Compression::None);
        assert_eq!(header.tile_compression, Compression::Gzip);
        assert_eq!(header.tile_type, TileType::Jpeg);
        assert_eq!(header.min_zoom, 12);
        assert_eq!(header.max_zoom, 13);
        assert_relative_eq!(header.min_position.lng, 0.0000014);
        assert_relative_eq!(header.min_position.lat, 0.0000015);
        assert_relative_eq!(header.max_position.lng, 0.0000016);
        assert_relative_eq!(header.max_position.lat, 0.0000017);
        assert_eq!(header.center_zoom, 14);
        assert_relative_eq!(header.center_position.lng, 0.0000018);
        assert_relative_eq!(header.center_position.lat, 0.0000019);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_fixture_header() {
        let bytes = include_bytes!("../testdata/test_fixture_1.pmtiles");

        let header = Header::decode(bytes[0..127].try_into().unwrap()).unwrap();

        assert_eq!(header.root_directory_offset, 127);
        assert_eq!(header.root_directory_length, 25);
        assert_eq!(header.metadata_offset, 152);
        assert_eq!(header.metadata_length, 247);
        assert_eq!(header.leaf_directories_offset, 0);
        assert_eq!(header.leaf_directories_length, 0);
        assert_eq!(header.tile_data_offset, 399);
        assert_eq!(header.tile_data_length, 69);
        assert_eq!(header.number_of_addressed_tiles, 1);
        assert_eq!(header.number_of_tile_entries, 1);
        assert_eq!(header.number_of_tile_contents, 1);
        assert_eq!(header.clustered, Clustered::NotClustered);
        assert_eq!(header.internal_compression, Compression::Gzip);
        assert_eq!(header.tile_compression, Compression::Gzip);
        assert_eq!(header.tile_type, TileType::Mvt);
        assert_eq!(header.min_zoom, 0);
        assert_eq!(header.max_zoom, 0);
        assert_relative_eq!(header.min_position.lng, 0.0);
        assert_relative_eq!(header.min_position.lat, 0.0);
        assert_relative_eq!(header.max_position.lng, 0.9999999);
        assert_relative_eq!(header.max_position.lat, 1.0);
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_invalid_magic_number() {
        let bytes = include_bytes!("../testdata/invalid.pmtiles");

        let result = Header::decode(bytes[0..127].try_into().unwrap());

        assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_decode_future_spec_version() {
        let bytes = include_bytes!("../testdata/invalid_v4.pmtiles");

        let result = Header::decode(bytes[0..127].try_into().unwrap());

        assert!(matches!(result, Err(PMTilesError::HeaderDecodeError(_))));
    }
}
