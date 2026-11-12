use secrecy::SecretString;
use serde::Serialize;

use crate::error::AppError;

/// Mirrors the `CHECK (purpose IN (...))` constraint on `auth_tokens`.
/// Keep the two in sync: adding a variant here means a migration there.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    strum::Display,
    strum::EnumString,
    sqlx::Type,
)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub(crate) enum TokenPurpose {
    VerifyEmail,
    VerifyPhone,
    ResetPassword,
    MagicLink,
    Invite,
    ChangeEmail,
}

/// A source of cryptographically random bytes.
///
/// Abstracted so tests can inject deterministic randomness and so we
/// don't hard-code the RNG choice across the codebase.
pub(crate) trait Rng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), AppError>;
}

/// Default OS-backed RNG.
pub(crate) struct OsRng;

impl Rng for OsRng {
    #[inline]
    fn fill(&self, buf: &mut [u8]) -> Result<(), AppError> {
        getrandom::fill(buf).map_err(|e| AppError::Internal(Box::new(e)))
    }
}

/// 32 bytes = 256 bits of entropy, hex-encoded to 64 ASCII chars.
const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// A token is a short human-readable prefix followed by 256 bits of
/// randomness, hex encoded.
///
/// Contract: the `Debug` impl *must not* leak the secret material — only
/// the prefix is printed. Use [`SecretString::expose_secret`] to read it.
pub(crate) trait RandomToken: Sized {
    const PREFIX: &'static str;

    /// Length of `PREFIX`, cached at compile time.
    #[expect(dead_code, reason = "Might be needed later")]
    const PREFIX_LEN: usize = Self::PREFIX.len();

    /// Generate a fresh token using the OS RNG.
    fn generate() -> Result<Self, AppError> {
        Self::generate_with(&OsRng)
    }

    /// Generate using an arbitrary RNG (useful for testing).
    fn generate_with(rng: &impl Rng) -> Result<Self, AppError>;

    /// Build from an already-generated `SecretString`. Callers are
    /// responsible for ensuring the value is well-formed; this exists so
    /// the macro can share the construction path.
    #[expect(dead_code, reason = "Might be needed later")]
    fn from_secret(secret: SecretString) -> Self;
}

/// Format `prefix + hex(32 random bytes)` into a `SecretString`.
pub(crate) fn generate_secret(prefix: &str, rng: &impl Rng) -> Result<SecretString, AppError> {
    let mut buf = [0u8; TOKEN_BYTES];
    rng.fill(&mut buf)?;

    // Pre-size the allocation: prefix + 64 hex chars, no reallocation.
    let mut out = String::with_capacity(prefix.len().saturating_add(TOKEN_HEX_LEN));
    out.push_str(prefix);
    hex::encode_to_slice(buf, &mut [0u8; TOKEN_HEX_LEN])
        .map_err(|e| AppError::Internal(Box::new(e)))?;
    // SAFETY-FREE variant: build the hex into a stack buffer, then push.
    let mut hex_buf = [0u8; TOKEN_HEX_LEN];
    hex::encode_to_slice(buf, &mut hex_buf).map_err(|e| AppError::Internal(Box::new(e)))?;
    out.push_str(std::str::from_utf8(&hex_buf).map_err(|e| AppError::Internal(Box::new(e)))?);

    Ok(SecretString::new(out.into_boxed_str()))
}

/// Declares a typed random token: a newtype over [`SecretString`] with a
/// fixed prefix and a `RandomToken` implementation.
///
/// ```ignore
/// random_token! {
///     /// Email-verification token.
///     pub(crate) struct VerifyToken = "vt_";
/// }
/// ```
macro_rules! random_token {
    ($(#[$meta:meta])* $vis:vis struct $name:ident = $prefix:literal;) => {
        $(#[$meta])*
        $vis struct $name(::secrecy::SecretString);

        impl $crate::auth::RandomToken for $name {
            const PREFIX: &'static str = $prefix;

            #[inline]
            fn generate_with(rng: &impl $crate::auth::Rng) -> ::core::result::Result<Self, $crate::error::AppError> {
                $crate::auth::generate_secret(Self::PREFIX, rng).map(Self)
            }

            #[inline]
            fn from_secret(secret: ::secrecy::SecretString) -> Self {
                Self(secret)
            }
        }

        #[allow(dead_code, reason = "Might be needed later")]
        impl $name {
            /// Generate a fresh token using the OS RNG.
            #[inline]
            $vis fn new() -> ::core::result::Result<Self, $crate::error::AppError> {
                <Self as $crate::auth::RandomToken>::generate()
            }

            /// Generate using an arbitrary RNG (testing only).
            #[inline]
            #[cfg(test)]
            $vis fn new_with(rng: &impl $crate::auth::Rng) -> ::core::result::Result<Self, $crate::error::AppError> {
                <Self as $crate::auth::RandomToken>::generate_with(rng)
            }
        }

        impl ::core::convert::TryFrom<String> for $name {
            type Error = AppError;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                if !s.starts_with(Self::PREFIX) {
                    return Err(AppError::invalid("invalid_token", "Invalid token format".to_owned()))
                }
                Ok(Self(s.into()))
            }
        }

        impl std::str::FromStr for $name {
            type Err = AppError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if !value.starts_with(Self::PREFIX) {
                    return Err(AppError::invalid("invalid_token", "Invalid token format".to_owned()))
                }
                Ok(Self(value.into()))
            }
        }


        impl ::core::ops::Deref for $name {
            type Target = ::secrecy::SecretString;

            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        // Do not derive Debug — we deliberately redact the secret.
        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("prefix", &Self::PREFIX)
                    .field("secret", &"<redacted>")
                    .finish()
            }
        }
    };
}

random_token! {
    /// Token emailed to a user to verify their address.
    pub(crate) struct VerifyToken = "vt_";
}

random_token! {
    pub(crate) struct SessionToken = "st_";
}

random_token! {
    /// Token used for password reset flows.
    pub(crate) struct ResetToken = "rt_";
}

random_token! {
    /// Token used for password reset flows.
    pub(crate) struct ApiKeyToken = "pk_";
}
