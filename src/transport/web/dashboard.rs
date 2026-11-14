//! Archive dashboard: totals, recent links, queue and tags.

use askama::Template;
use axum::extract::State;

use crate::{
    app::AppState,
    db::page::stats,
    identity::{AuthUser, Permission},
    prelude::*,
    service,
    transport::{
        htmx::Render,
        web::{
            library,
            viewmodels::archive::{LinkCard, PendingView, StatCard, TagRow},
            viewmodels::{NavGroup, NavItem, NavProvider, Section, Shell, icons},
        },
    },
    util::limit::Limit,
};

#[derive(Debug, Template)]
#[template(path = "archive/dashboard.templ.html")]
pub(super) struct DashboardTpl {
    shell: Shell,
    cards: Vec<StatCard>,
    recent: Vec<LinkCard>,
    pending: Vec<PendingView>,
    tags: Vec<TagRow>,
    can_write: bool,
}

pub(crate) struct LibraryDashboard;

impl NavProvider for LibraryDashboard {
    fn groups(
        user: &crate::identity::AuthUser,
        active: Section,
    ) -> Vec<super::viewmodels::NavGroup> {
        let item = |section: Section,
                    href: &'static str,
                    label: &'static str,
                    icon: &'static str| NavItem {
            href,
            label,
            icon,
            active: section == active,
        };

        if user.can(Permission::ARCHIVE_READ) {
            vec![
                (NavGroup {
                    label: "Library",
                    items: vec![
                        item(Section::Archive, "/", "Web archive", icons::GRID),
                        item(Section::Links, "/links", "All links", icons::LINK),
                        item(Section::Pinned, "/links/pinned", "Pinned", icons::PIN),
                        item(
                            Section::Unorganized,
                            "/links/unorganized",
                            "Unorganized",
                            icons::INBOX,
                        ),
                        item(
                            Section::Collections,
                            "/collections",
                            "Collections",
                            icons::FOLDERS,
                        ),
                        item(Section::Pending, "/pending", "Pending", icons::CLOCK),
                        item(Section::Tags, "/tags", "Tags", icons::TAG),
                    ],
                }),
            ]
        } else {
            vec![]
        }
    }
}

pub(super) async fn page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Render<DashboardTpl>> {
    let totals = stats(&state.db, user.id).await?;
    let recent = service::page::list(
        &state,
        &user,
        &service::page::PageListQuery {
            limit: Limit::from_const(6),
            ..Default::default()
        },
    )
    .await?;

    let pending =
        service::pending::list(&state.db, &user, None, None, Some(Limit::from_const(5))).await?;

    let tags = service::tag::list(&state.db, &user)
        .await?
        .into_iter()
        .map(TagRow::from)
        .collect();

    Ok(Render(DashboardTpl {
        shell: Shell::new::<LibraryDashboard>(&user, Section::Archive),
        cards: vec![
            StatCard::new("Links", totals.pages, "/links"),
            StatCard::new("Collections", totals.collections, "/collections"),
            StatCard::new("Tags", totals.tags, "/tags"),
            StatCard::new("Pinned", totals.pinned, "/links/pinned"),
        ],
        recent: library::cards(recent.items),
        pending: pending.items.into_iter().map(PendingView::from).collect(),
        tags,
        can_write: user.can(Permission::ARCHIVE_WRITE),
    }))
}
