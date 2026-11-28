//! Roles, when the reverse proxy says who's who: *modelers* build, save and
//! publish models; *deciders* only use what's published, building scenarios
//! and comparing them on the Simulation page. Logins stay in the proxy, which
//! sets a header with the user's groups (`--role-header`); a user in one of
//! the modeler groups (`--modelers`) is a modeler, anyone else a decider.
//! Without `--role-header`, everyone is a modeler, as before.

use axum::http::{HeaderMap, HeaderName, Method};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Modeler,
    Decider,
}

#[derive(Debug, Clone)]
pub struct Roles {
    /// The header the proxy sets with the user's groups (or role).
    pub header: HeaderName,
    /// The values of it that make someone a modeler.
    pub modelers: Vec<String>,
}

impl Roles {
    /// Groups are separated by commas, semicolons or spaces, and compared
    /// ignoring case. A header that appears twice counts as one list.
    pub fn role(&self, headers: &HeaderMap) -> Role {
        let modeler = headers
            .get_all(&self.header)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split([',', ';', ' ']))
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .any(|g| self.modelers.iter().any(|m| m.eq_ignore_ascii_case(g)));
        if modeler {
            Role::Modeler
        } else {
            Role::Decider
        }
    }
}

/// Whether a decider may make this request. Runs are checked again by the
/// handler: a decider may only run a published version, unchanged.
pub fn decider_may(method: &Method, path: &str) -> bool {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match *method {
        Method::GET | Method::HEAD => path != "/modeling",
        Method::POST => {
            matches!(
                segments.as_slice(),
                ["api", "v1", "comparisons"] | ["api", "v1", "runs"]
            ) || matches!(
                segments.as_slice(),
                ["api", "v1", "projects", _, "scenarios"]
            )
        }
        Method::PUT | Method::DELETE => {
            matches!(
                segments.as_slice(),
                ["api", "v1", "projects", _, "scenarios", _]
            )
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_from_the_header() {
        let roles = Roles {
            header: HeaderName::from_static("x-forwarded-groups"),
            modelers: vec!["engineering".into(), "simulation-modelers".into()],
        };
        let role = |value: Option<&str>| {
            let mut headers = HeaderMap::new();
            if let Some(v) = value {
                headers.insert("x-forwarded-groups", v.parse().unwrap());
            }
            roles.role(&headers)
        };
        assert_eq!(role(Some("staff, Engineering")), Role::Modeler);
        assert_eq!(role(Some("simulation-modelers")), Role::Modeler);
        assert_eq!(role(Some("staff;managers")), Role::Decider);
        assert_eq!(role(Some("engineering-interns")), Role::Decider);
        assert_eq!(role(None), Role::Decider);
    }

    #[test]
    fn what_deciders_may_do() {
        let may = |m: Method, p: &str| decider_may(&m, p);
        assert!(may(Method::GET, "/simulation"));
        assert!(may(Method::GET, "/api/v1/projects/line"));
        assert!(!may(Method::GET, "/modeling"));
        assert!(may(Method::POST, "/api/v1/comparisons"));
        assert!(may(Method::POST, "/api/v1/runs"));
        assert!(may(Method::POST, "/api/v1/projects/line/scenarios"));
        assert!(may(Method::PUT, "/api/v1/projects/line/scenarios/s1"));
        assert!(may(Method::DELETE, "/api/v1/projects/line/scenarios/s1"));
        for (m, p) in [
            (Method::POST, "/api/v1/projects"),
            (Method::PATCH, "/api/v1/projects/line"),
            (Method::DELETE, "/api/v1/projects/line"),
            (Method::POST, "/api/v1/projects/line/save"),
            (Method::POST, "/api/v1/projects/line/publish"),
            (Method::POST, "/api/v1/projects/line/check"),
            (Method::POST, "/api/v1/projects/line/archive"),
            (Method::POST, "/api/v1/projects/line/restore"),
            (Method::DELETE, "/api/v1/runs/1-2"),
        ] {
            assert!(!may(m.clone(), p), "{m} {p}");
        }
    }
}
