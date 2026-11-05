// SPDX-License-Identifier: AGPL-3.0-only
//! Places from Threema's point-of-interest server, for choosing a place to send (Android
//! `location/PoiRepository.kt`, URLs from `app/build.gradle.kts`): nearby places around a position,
//! and places by name, nearest first as the server gives them.
//!
//! The server gets the position and what was typed; both travel in the URL's path, so neither may
//! reach a log. Errors come from [`crate::https`], which strips URLs.

use std::time::Duration;

use libthreema::https::{HttpsMethod, HttpsRequest};
use serde_json::{Value, json};

use crate::{app::ApiError, https::HttpsClient};

const SERVER: &str = "https://poi.threema.ch";
/// Android's `NEARBY_RADIUS`.
const NEARBY_RADIUS_M: u32 = 750;
/// Android's `MAX_POI_COUNT`, for both lists.
const MAX_PLACES: usize = 30;
/// Android's `QUERY_MIN_LENGTH`.
const QUERY_MIN_CHARS: usize = 3;
const QUERY_MAX_CHARS: usize = 200;
const NAME_MAX_CHARS: usize = 256;
const MAX_ANSWER: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);

/// What makes something a place in Android's nearby list (`parseNearbyPoiType`).
const PLACE_KINDS: &[&str] = &[
    "amenity",
    "shop",
    "tourism",
    "sport",
    "leisure",
    "natural",
    "public_transport",
    "aeroway",
    "aerialway",
    "highway",
];

/// A position searched around.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Center {
    latitude: f64,
    longitude: f64,
}

impl Center {
    /// `None` outside the globe, and for 0/0, which is what a missing position looks like and which
    /// Android doesn't ask about either.
    pub(crate) fn new(latitude: f64, longitude: f64) -> Option<Self> {
        let on_globe = (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude);
        (on_globe && !(latitude == 0.0 && longitude == 0.0)).then_some(Self {
            latitude,
            longitude,
        })
    }

    /// Metres to a point (haversine), for a nearby place that comes without `dist`: Android lists
    /// those too, as it doesn't read the distance of this answer.
    fn distance_to(self, latitude: f64, longitude: f64) -> f64 {
        const EARTH_RADIUS_M: f64 = 6_371_000.0;
        let (from, to) = (self.latitude.to_radians(), latitude.to_radians());
        let half_north = (to - from) / 2.0;
        let half_east = (longitude - self.longitude).to_radians() / 2.0;
        let a = half_north.sin().powi(2) + from.cos() * to.cos() * half_east.sin().powi(2);
        2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
    }
}

/// `URLEncoder.encode(query, "UTF-8")` with `%20` for a space, as Android sends it. Every slash
/// and question mark ends up encoded, so a query stays one segment of the path.
fn encode_query(query: &str) -> String {
    let mut encoded = String::with_capacity(query.len());
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'*' | b'_') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn nearby_url(center: Center) -> String {
    format!(
        "{SERVER}/around/{:.6}/{:.6}/{NEARBY_RADIUS_M}/",
        center.latitude, center.longitude
    )
}

fn names_url(center: Center, query: &str) -> String {
    format!(
        "{SERVER}/names/{:.6}/{:.6}/{}/",
        center.latitude,
        center.longitude,
        encode_query(query)
    )
}

/// A name as one line of plain text: on the wire a location's name is a line of its own.
fn clean_name(name: &str) -> Option<String> {
    let line: Vec<&str> = name
        .split(|c: char| c.is_control() || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    let name: String = line.join(" ").chars().take(NAME_MAX_CHARS).collect();
    (!name.is_empty()).then_some(name)
}

fn place_position(place: &Value) -> Option<(String, f64, f64)> {
    let name = clean_name(place["name"].as_str()?)?;
    let latitude = place["lat"]
        .as_f64()
        .filter(|l| (-90.0..=90.0).contains(l))?;
    let longitude = place["lon"]
        .as_f64()
        .filter(|l| (-180.0..=180.0).contains(l))?;
    Some((name, latitude, longitude))
}

fn places_json(answer: &[u8], row: impl Fn(&Value) -> Option<Value>) -> Result<Value, ApiError> {
    let answer: Value = serde_json::from_slice(answer)
        .map_err(|_| ApiError::ServerTrouble("unreadable answer from the place server".into()))?;
    let Some(places) = answer.as_array() else {
        return Err(ApiError::ServerTrouble(
            "unexpected answer from the place server".into(),
        ));
    };
    let places: Vec<Value> = places.iter().filter_map(row).take(MAX_PLACES).collect();
    Ok(json!({ "places": places }))
}

/// The answer of `/around/`: what isn't a place Android lists is left out.
fn parse_nearby(answer: &[u8], center: Center) -> Result<Value, ApiError> {
    places_json(answer, |place| {
        if !PLACE_KINDS.iter().any(|kind| place.get(kind).is_some()) {
            return None;
        }
        let (name, latitude, longitude) = place_position(place)?;
        let distance = place["dist"]
            .as_f64()
            .filter(|d| *d >= 0.0)
            .unwrap_or_else(|| center.distance_to(latitude, longitude))
            .round();
        Some(
            json!({ "name": name, "latitude": latitude, "longitude": longitude,
                     "distance": distance }),
        )
    })
}

/// The answer of `/names/`: Android leaves out what comes without a distance.
fn parse_names(answer: &[u8]) -> Result<Value, ApiError> {
    places_json(answer, |place| {
        let distance = place["dist"].as_f64().filter(|d| *d >= 0.0)?;
        let (name, latitude, longitude) = place_position(place)?;
        Some(
            json!({ "name": name, "latitude": latitude, "longitude": longitude,
                     "distance": distance.round() }),
        )
    })
}

/// `findPlaces`: `{places: [{name, latitude, longitude, distance}]}`. By name with a `query`, else
/// around `center`.
pub(crate) async fn find(
    https: &HttpsClient,
    latitude: f64,
    longitude: f64,
    query: Option<&str>,
) -> Result<Value, ApiError> {
    if !latitude.is_finite() || !longitude.is_finite() {
        return Err(ApiError::Internal("invalid position".into()));
    }
    let Some(center) = Center::new(latitude, longitude) else {
        return Ok(json!({ "places": [] }));
    };
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let url = match query {
        Some(query) if query.chars().count() < QUERY_MIN_CHARS => {
            return Ok(json!({ "places": [] }));
        }
        Some(query) if query.chars().count() > QUERY_MAX_CHARS => {
            return Err(ApiError::Internal("search text too long".into()));
        }
        Some(query) => names_url(center, query),
        None => nearby_url(center),
    };
    let response = https
        .send_limited(
            HttpsRequest {
                timeout: TIMEOUT,
                url,
                method: HttpsMethod::Get,
                headers: Vec::new(),
                body: Vec::new(),
            },
            MAX_ANSWER,
        )
        .await
        .map_err(|error| ApiError::Network(error.to_string()))?;
    if !(200..300).contains(&response.status) {
        return Err(ApiError::ServerTrouble(format!(
            "place server answered {}",
            response.status
        )));
    }
    if query.is_some() {
        parse_names(&response.body)
    } else {
        parse_nearby(&response.body, center)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center() -> Center {
        Center::new(47.220087, 8.808609).unwrap()
    }

    // Android's MapPoiAroundUrlTest and MapPoiNamesUrlTest.
    #[test]
    fn urls_are_androids() {
        assert_eq!(
            nearby_url(center()),
            "https://poi.threema.ch/around/47.220087/8.808609/750/"
        );
        assert_eq!(
            names_url(center(), "Zürich City"),
            "https://poi.threema.ch/names/47.220087/8.808609/Z%C3%BCrich%20City/"
        );
    }

    #[test]
    fn urls_only_ever_name_threemas_server() {
        assert!(crate::https::is_threema_url(&nearby_url(center())));
        for query in ["@example.org", "a/../../b", "x?y#z", "...", "%2e%2e"] {
            let url = names_url(center(), query);
            let parsed = reqwest::Url::parse(&url).unwrap();
            assert_eq!(parsed.host_str(), Some("poi.threema.ch"), "{query}");
            assert_eq!(parsed.query(), None, "{query}");
            assert_eq!(parsed.fragment(), None, "{query}");
            // The query stays one segment: /names/lat/lon/query/
            assert_eq!(parsed.path_segments().unwrap().count(), 5, "{query}");
        }
    }

    #[test]
    fn a_missing_or_impossible_position_is_no_center() {
        assert!(Center::new(0.0, 0.0).is_none());
        assert!(Center::new(90.5, 1.0).is_none());
        assert!(Center::new(1.0, -180.5).is_none());
        assert!(Center::new(f64::NAN, 1.0).is_none());
        assert!(Center::new(0.0, 1.0).is_some());
    }

    // Android's test/resources/api/responses, as in its PoiRepositoryTest.
    #[test]
    fn androids_nearby_answer() {
        let answer = include_bytes!("../tests/fixtures/poi-nearby.json");
        let center = Center::new(47.268, 8.715).unwrap();
        let places = parse_nearby(answer, center).unwrap();
        let places = places["places"].as_array().unwrap();
        assert_eq!(places.len(), 15);
        assert_eq!(
            places[0],
            json!({"name": "Rathaus", "latitude": 47.2690785, "longitude": 8.7142272,
                   "distance": 64.0})
        );
        assert_eq!(places[5]["name"], "Waldhüüsli");
        assert_eq!(places[8]["name"], "Bergli");
        assert_eq!(places[14]["name"], "Threema Shop");
        assert_eq!(places[14]["distance"], 622.0);
    }

    #[test]
    fn androids_names_answer() {
        let answer = include_bytes!("../tests/fixtures/poi-names.json");
        let places = parse_names(answer).unwrap();
        assert_eq!(
            places["places"],
            json!([
                {"name": "POI 1", "latitude": 47.5, "longitude": 8.1, "distance": 20.0},
                {"name": "POI 2", "latitude": 47.1, "longitude": 8.0, "distance": 25.0},
                {"name": "POI 3", "latitude": 47.1, "longitude": 8.0, "distance": 30.0},
            ])
        );
    }

    #[test]
    fn nearby_answer_gives_named_places_with_their_distance() {
        let answer = br#"[
            {"id": 1, "name": "Cafe", "lat": 47.221, "lon": 8.808609, "amenity": "cafe"},
            {"id": 2, "name": "Summit", "lat": 47.22, "lon": 8.81, "natural": "peak", "dist": 99},
            {"id": 3, "name": "No kind", "lat": 47.22, "lon": 8.81},
            {"id": 4, "lat": 47.22, "lon": 8.81, "shop": "kiosk"},
            {"id": 5, "name": "Off the globe", "lat": 91.0, "lon": 8.81, "shop": "kiosk"},
            {"id": 6, "name": "  ", "lat": 47.22, "lon": 8.81, "shop": "kiosk"}
        ]"#;
        let places = parse_nearby(answer, center()).unwrap();
        let places = places["places"].as_array().unwrap();
        assert_eq!(places.len(), 2, "{places:?}");
        assert_eq!(places[0]["name"], "Cafe");
        assert_eq!(places[0]["latitude"], 47.221);
        // Without `dist`: 0.000913 degrees north is about a hundred metres.
        assert_eq!(places[0]["distance"], 102.0);
        assert_eq!(places[1]["name"], "Summit");
        assert_eq!(places[1]["distance"], 99.0);
    }

    #[test]
    fn names_answer_needs_a_distance() {
        let answer = br#"[
            {"name": "Main Street", "lat": 47.2, "lon": 8.8, "dist": 1234.4, "highway": "primary"},
            {"name": "Elsewhere", "lat": 47.3, "lon": 8.9, "place": "town"}
        ]"#;
        let places = parse_names(answer).unwrap();
        let places = places["places"].as_array().unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0]["name"], "Main Street");
        assert_eq!(places[0]["distance"], 1234.0);
    }

    #[test]
    fn a_name_stays_one_line_of_bounded_length() {
        assert_eq!(clean_name(" A\nB\r\n\tC\u{0}D ").unwrap(), "A B C D");
        assert_eq!(clean_name("\n \t"), None);
        let long = "x".repeat(NAME_MAX_CHARS + 10);
        assert_eq!(clean_name(&long).unwrap().chars().count(), NAME_MAX_CHARS);
    }

    #[test]
    fn lists_are_bounded_and_broken_answers_are_errors() {
        let many: Vec<Value> = (0..100)
            .map(|_| json!({"name": "P", "lat": 1.0, "lon": 1.0, "dist": 1}))
            .collect();
        let answer = serde_json::to_vec(&many).unwrap();
        let places = parse_names(&answer).unwrap();
        assert_eq!(places["places"].as_array().unwrap().len(), MAX_PLACES);

        for broken in [&b"not json"[..], &b"{\"error\": 1}"[..]] {
            assert!(matches!(
                parse_names(broken),
                Err(ApiError::ServerTrouble(_))
            ));
        }
    }

    // What needs no answer doesn't go out: with the client stalled, a request would never return.
    #[tokio::test]
    async fn short_queries_and_missing_positions_stay_on_the_device() {
        let https = HttpsClient::new().unwrap();
        crate::https::stall_requests();
        let empty = json!({ "places": [] });
        assert_eq!(find(&https, 0.0, 0.0, None).await.unwrap(), empty);
        assert_eq!(find(&https, 47.2, 8.8, Some(" ab ")).await.unwrap(), empty);
        assert!(find(&https, f64::NAN, 8.8, None).await.is_err());
        let long = "x".repeat(QUERY_MAX_CHARS + 1);
        assert!(find(&https, 47.2, 8.8, Some(&long)).await.is_err());
    }
}
