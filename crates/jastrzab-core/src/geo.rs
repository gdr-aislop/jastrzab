//! A position fix as the rest of the core sees it, and the one calculation on positions it needs.

use hauk::Provider;

/// One location update, already normalised from whatever the platform reported.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    /// Horizontal accuracy in metres; negative when unknown (GeoClue reports -1).
    pub accuracy_m: f64,
    /// Metres per second, when the source knows it.
    pub speed: Option<f32>,
    /// The source's own description. GeoClue says `"GPS GGA"`/`"GPS RMC"` for satellite fixes.
    pub description: String,
    /// When the position was determined, as Unix seconds.
    pub unix_time: f64,
}

impl Fix {
    /// Whether the fix came from satellites rather than the network. A hint only: GeoClue
    /// documents `Description` as human-readable text.
    pub fn is_gps(&self) -> bool {
        self.description.trim_start().get(..3).is_some_and(|p| p.eq_ignore_ascii_case("gps"))
    }

    /// The `prv` value the Hauk viewer uses to colour the marker.
    pub fn provider(&self) -> Provider {
        if self.is_gps() { Provider::Fine } else { Provider::Coarse }
    }
}

/// Great-circle distance in metres between two `(latitude, longitude)` points in degrees.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (lat1, lat2) = (a.0.to_radians(), b.0.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (b.1 - a.1).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(description: &str) -> Fix {
        Fix { latitude: 0.0, longitude: 0.0, accuracy_m: 3.0, speed: None, description: description.into(), unix_time: 0.0 }
    }

    #[test]
    fn gps_is_recognised_from_the_description() {
        assert!(fix("GPS GGA").is_gps());
        assert!(fix("gps rmc").is_gps());
        assert!(!fix("WiFi").is_gps());
        assert!(!fix("").is_gps());
        assert_eq!(fix("GPS GGA").provider(), Provider::Fine);
        assert_eq!(fix("3G").provider(), Provider::Coarse);
    }

    #[test]
    fn distance_is_about_right() {
        // One degree of latitude is about 111.2 km.
        let d = distance_m((10.0, 20.0), (11.0, 20.0));
        assert!((d - 111_195.0).abs() < 300.0, "{d}");
        assert_eq!(distance_m((1.0, 2.0), (1.0, 2.0)), 0.0);
        // Antipodal points: half the circumference.
        assert!((distance_m((0.0, 0.0), (0.0, 180.0)) - 20_015_087.0).abs() < 1000.0);
    }
}
