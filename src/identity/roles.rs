use crate::{error::ForbiddenReason, identity::AuthUser, prelude::*};

pub(crate) const SUPER_ADMIN_ROLE: &str = "super_admin";
pub(crate) const DEFAULT_ROLE: &str = "user";

/// Only a wildcard holder may hand out or take away the top role.
pub(crate) fn ensure_role_assignable(actor: &AuthUser, role: &str) -> Result<()> {
    if role == SUPER_ADMIN_ROLE && !actor.is_super_admin() {
        return Err(AppError::Forbidden(ForbiddenReason::MissingPermission));
    }
    Ok(())
}

pub(crate) fn ensure_not_self(actor_id: i64, target_id: i64, message: &'static str) -> Result<()> {
    if actor_id == target_id {
        Err(AppError::BadRequest(message.into()))
    } else {
        Ok(())
    }
}
