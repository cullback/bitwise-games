//! Hand-authored level for the slice-1 BSP demo: one convex hexagonal
//! sector with six angled walls. No portals yet.
//!
//! All world coordinates are in fp 256 — same scale as the wolfenstein
//! map. The world spans (0, 0)..(4096, 4096) so the renderer can share
//! the same 12-bit player position bit layout.
//!
//! Vertices are listed CCW around the room in math-convention space
//! (+y north). Linedefs walk the boundary CW so the sector interior
//! sits on the RIGHT of each linedef — same convention as Doom.

use bitwise_games::draw_command::{Color, DARK_GREY, LIGHT_GREY, ORANGE, PINK, RED, WHITE};

// Near-plane in view-space depth: walls clipped to this before
// projection so the perspective divide stays bounded. Kept very small
// — at ≥4 fp the projection covers walls passing close to the side
// of the player. Bigger values crop off the wrap-around portion of a
// near wall and the renderer ends up skipping it, which the player
// reads as a "see through the wall" bug.
pub const NEAR_DEPTH: i32 = 4;

// Sector vertical geometry, fp 256. One world unit = 256 fp.
pub const WALL_TOP: i32 = 256;
pub const WALL_BOT: i32 = 0;
pub const EYE_HEIGHT: i32 = 128;

// Approximate collision radius. The cross-product test in `point_inside`
// rejects positions where p is within `PLAYER_RADIUS` of any linedef.
// The test compares `cross` to `-PLAYER_RADIUS * EDGE_LEN`, and our
// hexagon edges all have length ≈ 512 fp, so this approximation is
// exact for the slice-1 level. Real levels with mixed edge lengths
// would normalise per linedef.
const PLAYER_RADIUS: i32 = 16;
const EDGE_LEN_APPROX: i32 = 512;
const CROSS_THRESHOLD: i32 = -PLAYER_RADIUS * EDGE_LEN_APPROX;

#[rustfmt::skip]
pub const VERTICES: &[(i32, i32)] = &[
    // Hexagon, centre (2048, 2048), radius 512. CCW from east.
    (2560, 2048), // V0  east
    (2304, 2491), // V1  NE
    (1792, 2491), // V2  NW
    (1536, 2048), // V3  west
    (1792, 1605), // V4  SW
    (2304, 1605), // V5  SE
];

pub struct Linedef {
    pub v0: u8,
    pub v1: u8,
    pub color: Color,
}

// CW around the hexagon. Each wall gets a distinct colour so the
// geometry reads immediately: this is the visual proof the angled-wall
// projection works, before we layer textures / materials on top.
pub const LINEDEFS: &[Linedef] = &[
    Linedef {
        v0: 0,
        v1: 5,
        color: WHITE,
    }, // E   → SE
    Linedef {
        v0: 5,
        v1: 4,
        color: LIGHT_GREY,
    }, // SE  → SW
    Linedef {
        v0: 4,
        v1: 3,
        color: DARK_GREY,
    }, // SW  → W
    Linedef {
        v0: 3,
        v1: 2,
        color: RED,
    }, // W   → NW
    Linedef {
        v0: 2,
        v1: 1,
        color: ORANGE,
    }, // NW  → NE
    Linedef {
        v0: 1,
        v1: 0,
        color: PINK,
    }, // NE  → E
];

/// True if `p` fits inside the (convex) sector polygon with at least
/// `PLAYER_RADIUS` of clearance on every wall. For each linedef the
/// cross product `ex * (p.y − v0.y) − ey * (p.x − v0.x)` equals twice
/// the signed area of (v0, v1, p) — negative on the interior side,
/// magnitude proportional to perpendicular distance × edge length.
pub fn point_inside(p: (i32, i32)) -> bool {
    for ld in LINEDEFS {
        let (v0x, v0y) = VERTICES[ld.v0 as usize];
        let (v1x, v1y) = VERTICES[ld.v1 as usize];
        let ex = v1x - v0x;
        let ey = v1y - v0y;
        let cross = ex * (p.1 - v0y) - ey * (p.0 - v0x);
        if cross >= CROSS_THRESHOLD {
            return false;
        }
    }
    true
}
