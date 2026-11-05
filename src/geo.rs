//! Geo helpers: point-in-polygon tests for GeoJSON geofences.

use serde_json::Value;

/// Returns true if the given lat/lon falls inside a GeoJSON Feature/Geometry
/// of type Polygon or MultiPolygon.
///
/// GeoJSON coordinates are stored as [longitude, latitude].
pub fn point_in_geojson(lat: f64, lon: f64, geojson: &Value) -> bool {
    // Accept either a Feature (has "geometry") or a raw Geometry object.
    let geom = geojson.get("geometry").unwrap_or(geojson);

    match geom.get("type").and_then(|t| t.as_str()) {
        Some("Polygon") => geom
            .get("coordinates")
            .and_then(|c| c.as_array())
            .and_then(|rings| rings.first())
            .and_then(|ring| ring.as_array())
            .map(|ring| point_in_ring(lon, lat, ring))
            .unwrap_or(false),
        Some("MultiPolygon") => {
            if let Some(polys) = geom.get("coordinates").and_then(|c| c.as_array()) {
                for poly in polys {
                    if let Some(ring) = poly
                        .as_array()
                        .and_then(|rings| rings.first())
                        .and_then(|ring| ring.as_array())
                    {
                        if point_in_ring(lon, lat, ring) {
                            return true;
                        }
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Ray-casting point-in-polygon test against a single linear ring.
fn point_in_ring(x: f64, y: f64, ring: &[Value]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }

    let coord = |i: usize| -> (f64, f64) {
        let p = &ring[i];
        let px = p.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let py = p.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
        (px, py)
    };

    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = coord(i);
        let (xj, yj) = coord(j);
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn square_geojson() -> Value {
        // Square around Berlin: lon 13.40..13.42, lat 52.51..52.53
        json!({
            "type": "Feature",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [13.40, 52.51],
                    [13.42, 52.51],
                    [13.42, 52.53],
                    [13.40, 52.53],
                    [13.40, 52.51]
                ]]
            }
        })
    }

    #[test]
    fn point_inside_polygon() {
        let gf = square_geojson();
        assert!(point_in_geojson(52.52, 13.41, &gf)); // center
    }

    #[test]
    fn point_outside_polygon() {
        let gf = square_geojson();
        assert!(!point_in_geojson(52.60, 13.41, &gf)); // north of square
        assert!(!point_in_geojson(52.52, 13.50, &gf)); // east of square
    }

    #[test]
    fn multipolygon_supported() {
        let gf = json!({
            "type": "MultiPolygon",
            "coordinates": [[[
                [13.40, 52.51],
                [13.42, 52.51],
                [13.42, 52.53],
                [13.40, 52.53],
                [13.40, 52.51]
            ]]]
        });
        assert!(point_in_geojson(52.52, 13.41, &gf));
    }

    #[test]
    fn invalid_geometry_false() {
        let gf = json!({"type": "Point", "coordinates": [13.41, 52.52]});
        assert!(!point_in_geojson(52.52, 13.41, &gf));
    }
}
