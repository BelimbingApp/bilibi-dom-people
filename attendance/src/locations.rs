//! Clocking locations and the point-in-location check (Bilimbi
//! `Bilimbi.People.Attendance.ClockingLocation` and `Locations`): the
//! `people_attendance_clocking_locations` table. Private to the facade.
use crate::{shifts, AttendanceError, FieldError, Result};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// A validation bound, not policy: a radius beyond this is not a location.
pub const MAX_RADIUS_METERS: i32 = 100_000;
const EARTH_RADIUS_METERS: f64 = 6_371_008.8;

/// One clocking location: a centre and a radius in meters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClockingLocation {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub latitude: Decimal,
    pub longitude: Decimal,
    pub radius_meters: i32,
    pub status: String,
}
impl ClockingLocation {
    /// Great-circle distance in meters from the location's centre.
    pub fn distance_meters(&self, latitude: Decimal, longitude: Decimal) -> f64 {
        let (lat1, lon1) = (radians(self.latitude), radians(self.longitude));
        let (lat2, lon2) = (radians(latitude), radians(longitude));
        let a = ((lat2 - lat1) / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_METERS * a.sqrt().min(1.0).asin()
    }
    fn audit_values(&self) -> Value {
        json!({
            "code": self.code,
            "name": self.name,
            "latitude": self.latitude.to_string(),
            "longitude": self.longitude.to_string(),
            "radius_meters": self.radius_meters,
            "status": self.status,
        })
    }
}

fn radians(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0).to_radians()
}

/// What adds a clocking location.
#[derive(Clone, Debug)]
pub struct NewClockingLocation {
    pub code: String,
    pub name: String,
    pub latitude: Decimal,
    pub longitude: Decimal,
    pub radius_meters: i32,
}

/// The facts once validated.
pub(crate) struct Valid {
    pub code: String,
    pub name: String,
    pub latitude: Decimal,
    pub longitude: Decimal,
    pub radius_meters: i32,
}

/// Bilimbi `ClockingLocation.changeset/2`, in its words.
pub(crate) fn validate(new: &NewClockingLocation) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let (code, name) = shifts::code_and_name(&new.code, &new.name, &mut errors);
    if new.latitude < Decimal::from(-90) {
        errors.push(FieldError::new(
            "latitude",
            "must be greater than or equal to -90",
        ));
    } else if new.latitude > Decimal::from(90) {
        errors.push(FieldError::new(
            "latitude",
            "must be less than or equal to 90",
        ));
    }
    if new.longitude < Decimal::from(-180) {
        errors.push(FieldError::new(
            "longitude",
            "must be greater than or equal to -180",
        ));
    } else if new.longitude > Decimal::from(180) {
        errors.push(FieldError::new(
            "longitude",
            "must be less than or equal to 180",
        ));
    }
    if new.radius_meters <= 0 {
        errors.push(FieldError::new("radius_meters", "must be greater than 0"));
    } else if new.radius_meters > MAX_RADIUS_METERS {
        errors.push(FieldError::new(
            "radius_meters",
            "must be less than or equal to 100000",
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Valid {
        code,
        name,
        latitude: new.latitude.round_dp(6),
        longitude: new.longitude.round_dp(6),
        radius_meters: new.radius_meters,
    })
}

const COLUMNS: &str = "id, code, name, latitude, longitude, radius_meters, status";

fn read(row: &Row) -> ClockingLocation {
    ClockingLocation {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        latitude: row.get("latitude"),
        longitude: row.get("longitude"),
        radius_meters: row.get("radius_meters"),
        status: row.get("status"),
    }
}

/// The company's locations, active first then by code.
pub(crate) async fn list(
    client: &impl GenericClient,
    scope: &Scope,
    company_id: i64,
) -> Result<Vec<ClockingLocation>> {
    Ok(client
        .query(
            &format!("SELECT {COLUMNS} FROM people_attendance_clocking_locations WHERE tenant_id = $1 AND company_id = $2 ORDER BY status, code"),
            &[&scope.tenant(), &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect())
}

/// Bilimbi `Locations.locate/4` and `place_event/4`: the nearest active
/// location whose radius contains the point, or none; with no coordinates
/// or no containing location, the company's rule decides.
pub(crate) async fn place(
    client: &impl GenericClient,
    scope: &Scope,
    company_id: i64,
    point: Option<(Decimal, Decimal)>,
    location_required: bool,
) -> Result<Option<i64>> {
    let Some((latitude, longitude)) = point else {
        return if location_required {
            Err(AttendanceError::LocationRequired)
        } else {
            Ok(None)
        };
    };
    let nearest = list(client, scope, company_id)
        .await?
        .into_iter()
        .filter(|location| location.status == "active")
        .map(|location| {
            let distance = location.distance_meters(latitude, longitude);
            (location, distance)
        })
        .filter(|(location, distance)| *distance <= f64::from(location.radius_meters))
        .min_by(|(_, a), (_, b)| a.total_cmp(b));
    match nearest {
        Some((location, _)) => Ok(Some(location.id)),
        None if location_required => Err(AttendanceError::OutsideClockingLocation),
        None => Ok(None),
    }
}

/// A taken code, decided on the transaction before the insert.
pub(crate) async fn check_code(
    client: &impl GenericClient,
    company_id: i64,
    code: &str,
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_attendance_clocking_locations WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(AttendanceError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    Ok(())
}

/// Inserts the location, recorded as a created change.
pub(crate) async fn insert(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    valid: Valid,
) -> Result<ClockingLocation> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("INSERT INTO people_attendance_clocking_locations (tenant_id, company_id, code, name, latitude, longitude, radius_meters, status) \
                              VALUES ($1, $2, $3, $4, $5, $6, $7, 'active') RETURNING {COLUMNS}"),
                    &[
                        &tenant,
                        &company_id,
                        &valid.code,
                        &valid.name,
                        &valid.latitude,
                        &valid.longitude,
                        &valid.radius_meters,
                    ],
                )
                .await?;
            let location = read(&row);
            let change = Change {
                kind: "people_attendance_clocking_location".into(),
                id: location.id,
                old: json!({}),
                new: location.audit_values(),
            };
            Ok((location, vec![change]))
        })
    })
    .await
    .map_err(AttendanceError::from)
}

/// Sets the location's status, recorded as an updated change; a location
/// of another company or missing is `NotFound`.
pub(crate) async fn set_status(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    location_id: i64,
    status: &'static str,
) -> Result<ClockingLocation> {
    let current = tx
        .sql()
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_attendance_clocking_locations WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&scope.tenant(), &company_id, &location_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .ok_or(AttendanceError::NotFound)?;
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("UPDATE people_attendance_clocking_locations SET status = $2, updated_at = now() WHERE id = $1 RETURNING {COLUMNS}"),
                    &[&current.id, &status],
                )
                .await?;
            let location = read(&row);
            let change = Change {
                kind: "people_attendance_clocking_location".into(),
                id: location.id,
                old: current.audit_values(),
                new: location.audit_values(),
            };
            Ok((location, vec![change]))
        })
    })
    .await
    .map_err(AttendanceError::from)
}
