/// Functionality to implement the PMTiles V3 tile ID spec. Tiles are arranged into a pyramid, with
/// each successive zoom layer having 4x the tiles of the previous (2x as many in both the x and y
/// axis). In PMTiles, the tile ID is the index into the discrete 2D Hilbert space at that zoom
/// level, offset by the tiles at all previous zoom levels.
use wasm_bindgen::prelude::*;

use crate::{PMTilesError, PMTilesResult};

/// Maximum allowed zoom level. All tiles must have a zoom level equal or lower to this value. It
/// is the largest value at which no u64 overflow occurs for any tiles.
const MAX_ZOOM: u8 = 30;

/// Maximum tile ID. The final tile ID at zoom level 30.
///
/// This formula can be worked out from the well-known identify for the sum of a geometric series
/// (powers of 4).
const MAX_TILE_ID: u64 = (((1 << (2 * (MAX_ZOOM + 1))) - 1) / 3) - 1;

/// PMTiles V3 tile ID. Guaranteed to be valid.
#[wasm_bindgen]
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileID(u64);

#[wasm_bindgen]
impl TileID {
    /// Creates and checks a PMTiles V3 tile ID. Fails if the tile ID is too large.
    pub fn new(tile_id: u64) -> PMTilesResult<Self> {
        if tile_id > MAX_TILE_ID {
            return Err(PMTilesError::TileOverflowError);
        }
        Ok(Self(tile_id))
    }

    /// Returns the raw numeric tile ID.
    pub fn id(&self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for TileID {
    type Error = PMTilesError;

    fn try_from(value: u64) -> PMTilesResult<Self> {
        TileID::new(value)
    }
}

impl From<TileCoordinate> for TileID {
    fn from(value: TileCoordinate) -> Self {
        // Based on the well-known identity for the sum of a geometric series (powers of 4 here).
        let offset: u64 = ((1 << (2 * value.z)) - 1) / 3;
        let h = fast_hilbert::xy2h(value.x, value.y, value.z);
        Self(offset + h)
    }
}

/// PMTiles V3 tile coordinate. Guaranteed to be valid. Can be easily exchanged for a TileID.
#[wasm_bindgen]
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileCoordinate {
    z: u8,
    x: u32,
    y: u32,
}

#[wasm_bindgen]
impl TileCoordinate {
    /// Creates a new PMTiles tile coordinate. Fails if the zoom level is beyond the maximum.
    pub fn new(z: u8, x: u32, y: u32) -> PMTilesResult<Self> {
        if z > MAX_ZOOM {
            return Err(PMTilesError::TileOverflowError);
        }
        Ok(Self { z, x, y })
    }

    /// Creates a TileCoordinate from a TileID.
    pub fn from_tile_id(tile_id: TileID) -> Self {
        // Inspired by https://github.com/protomaps/go-pmtiles/blob/main/pmtiles/tile_id.go#L34.
        // This identity comes from some clever shenanigans with the sum of a geometric series
        // (powers of 4), and the bit representation for powers of 2.
        let z = ((64 - (3 * tile_id.id() + 1).leading_zeros() - 1) / 2) as u8;

        // z == 0 makes the Hilbert calculation panic.
        if z == 0 {
            return Self { z: 0, x: 0, y: 0 };
        }

        let offset: u64 = ((1 << (2 * z)) - 1) / 3;
        let (x, y) = fast_hilbert::h2xy(tile_id.id() - offset, z);
        Self { z, x, y }
    }

    /// Returns the zoom level z.
    pub fn z(&self) -> u8 {
        self.z
    }

    /// Returns the x coordinate.
    pub fn x(&self) -> u32 {
        self.x
    }

    /// Returns the z coordinate.
    pub fn y(&self) -> u32 {
        self.y
    }
}

impl From<TileID> for TileCoordinate {
    fn from(tile_id: TileID) -> Self {
        TileCoordinate::from_tile_id(tile_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case((0, 0, 0), TileID(0))]
    #[case((1, 0, 0), TileID(1))]
    #[case((1, 0, 1), TileID(2))]
    #[case((1, 1, 1), TileID(3))]
    #[case((1, 1, 0), TileID(4))]
    #[case((2, 0, 0), TileID(5))]
    fn test_zxy_to_tile_id(#[case] zxy: (u8, u32, u32), #[case] want: TileID) {
        let got: TileID = TileCoordinate::new(zxy.0, zxy.1, zxy.2).unwrap().into();
        assert_eq!(got, want);
    }

    #[rstest]
    #[case(TileID(0), (0, 0, 0))]
    #[case(TileID(1), (1, 0, 0))]
    #[case(TileID(2), (1, 0, 1))]
    #[case(TileID(3), (1, 1, 1))]
    #[case(TileID(4), (1, 1, 0))]
    #[case(TileID(5), (2, 0, 0))]
    fn test_tile_id_to_zxy(#[case] tile_id: TileID, #[case] zxy: (u8, u32, u32)) {
        let c = TileCoordinate::from_tile_id(tile_id);
        assert_eq!((c.z(), c.x(), c.y()), zxy);
    }

    #[test]
    fn test_tile_id_round_trip() {
        for z in 0..12 {
            for x in 0..(1 << z) {
                for y in 0..(1 << z) {
                    let tile_id: TileID = TileCoordinate::new(z, x, y).unwrap().into();
                    let got = TileCoordinate::from_tile_id(tile_id);

                    assert_eq!((got.z(), got.x(), got.y()), (z, x, y));
                }
            }
        }
    }

    #[test]
    fn test_tile_extremes() {
        for z in 0..(MAX_ZOOM + 1) {
            let max_dimension = 2_u32.pow(z.into()) - 1;

            // Top left
            let coordinate = TileCoordinate::new(z, 0, 0).unwrap();
            let tile_id: TileID = coordinate.into();
            assert!(TileID::new(tile_id.id()).is_ok());

            // Top right
            let coordinate = TileCoordinate::new(z, max_dimension, 0).unwrap();
            let tile_id: TileID = coordinate.into();
            assert!(TileID::new(tile_id.id()).is_ok());

            // Bottom left
            let coordinate = TileCoordinate::new(z, 0, max_dimension).unwrap();
            let tile_id: TileID = coordinate.into();
            assert!(TileID::new(tile_id.id()).is_ok());

            // Bottom right
            let coordinate = TileCoordinate::new(z, max_dimension, max_dimension).unwrap();
            let tile_id: TileID = coordinate.into();
            assert!(TileID::new(tile_id.id()).is_ok());
        }
    }

    #[test]
    fn test_tile_coordinate_overflow() {
        assert!(TileCoordinate::new(MAX_ZOOM + 1, 0, 0).is_err());
    }

    #[test]
    fn test_tile_id_overflow() {
        assert!(TileID::new(MAX_TILE_ID + 1).is_err());
    }

    #[test]
    fn test_max_tile_has_no_overflow() {
        let tile_id = TileID::new(MAX_TILE_ID).unwrap();
        let unchecked_coord = TileCoordinate::from_tile_id(tile_id);
        assert!(
            TileCoordinate::new(
                unchecked_coord.z(),
                unchecked_coord.x(),
                unchecked_coord.y()
            )
            .is_ok()
        );
    }
}
