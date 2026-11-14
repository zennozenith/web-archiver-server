use chrono::NaiveDate;

use crate::{prelude::*, util::time::today};

const NAME_MAX: usize = 100;
const BIO_MAX: usize = 1000;
const LOCALE_MAX: usize = 35;
const TIMEZONE_MAX: usize = 64;

/// Full replacement of the editable profile fields (the route is `PUT`).
/// `None`/blank clears `display_name`, names, bio and `birth_date`;
/// `None`/blank leaves locale and timezone unchanged.
#[derive(Debug, Default)]
pub(crate) struct ProfileUpdate {
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub bio: Option<String>,
    /// `YYYY-MM-DD`
    pub birth_date: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

impl ProfileUpdate {
    /// Trims, turns blanks into `None`, enforces lengths and the date format.
    pub(crate) fn normalize(self) -> Result<Self> {
        let birth_date = match clean("birth_date", self.birth_date, 10)? {
            Some(raw) => {
                let date = NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
                    .map_err(|_| AppError::BadRequest("birth_date must be YYYY-MM-DD".into()))?;
                if date > today() {
                    return Err(AppError::BadRequest(
                        "birth_date cannot be in the future".into(),
                    ));
                }
                Some(date.format("%Y-%m-%d").to_string()) // canonical form for the CHECK
            }
            None => None,
        };

        Ok(Self {
            display_name: clean("display_name", self.display_name, NAME_MAX)?,
            first_name: clean("first_name", self.first_name, NAME_MAX)?,
            last_name: clean("last_name", self.last_name, NAME_MAX)?,
            bio: clean("bio", self.bio, BIO_MAX)?,
            birth_date,
            locale: clean("locale", self.locale, LOCALE_MAX)?,
            timezone: clean("timezone", self.timezone, TIMEZONE_MAX)?,
        })
    }
}

fn clean(field: &str, value: Option<String>, max: usize) -> Result<Option<String>> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {max} characters"
        )));
    }
    Ok(Some(value.to_owned()))
}
