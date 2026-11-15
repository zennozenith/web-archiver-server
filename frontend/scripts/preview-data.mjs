/**
 * Placeholder data for the static preview.
 *
 * The shapes mirror the Askama context structs in `src/web/view.rs`,
 * `src/web/templates.rs`, and `src/web_archive/`. Keep them in sync when a
 * template starts using a new field.
 */

const none = () => ({ is_some: () => false });
const some = (value) => ({ is_some: () => true, value });

const format = (key, label, icon) => ({ key, label, icon });

export const formats = {
  readable: format("readable", "Readable", "📖"),
  webpage: format("webpage", "Webpage", "🌐"),
  screenshot: format("screenshot", "Screenshot", "🖼"),
  pdf: format("pdf", "PDF", "📄"),
};

export const tags = [
  { id: 1, name: "design", slug: "design", count: 4 },
  { id: 2, name: "rust", slug: "rust", count: 3 },
  { id: 3, name: "frontend", slug: "frontend", count: 2 },
  { id: 4, name: "reading", slug: "reading", count: 2 },
  { id: 5, name: "tools", slug: "tools", count: 1 },
  { id: 6, name: "css", slug: "css", count: 1 },
];

const tag = (name) => {
  const found = tags.find((entry) => entry.name === name);
  if (!found) throw new Error(`Unknown preview tag: ${name}`);
  return found;
};

export const collections = [
  {
    id: 1,
    name: "Design",
    emoji: "🎨",
    description: "Typography, colour systems, and interface details worth keeping.",
    color: "#6d5dfc",
    link_count: 3,
    public: true,
    parent_id: none(),
    share_url: "/public/collections/1",
  },
  {
    id: 2,
    name: "Development",
    emoji: "🧑‍💻",
    description: "Rust, web platform notes, and tools I return to.",
    color: "#0ea5e9",
    link_count: 2,
    public: false,
    parent_id: none(),
    share_url: "/public/collections/2",
  },
  {
    id: 3,
    name: "Reading list",
    emoji: "📚",
    description: "Long reads for a quiet afternoon.",
    color: "#f59e0b",
    link_count: 2,
    public: true,
    parent_id: none(),
    share_url: "/public/collections/3",
  },
  {
    id: 4,
    name: "Recipes",
    emoji: "🍲",
    description: "Things to cook again.",
    color: "#22c55e",
    link_count: 1,
    public: false,
    parent_id: none(),
    share_url: "/public/collections/4",
  },
  {
    id: 5,
    name: "Design systems",
    emoji: "🧩",
    description: "A sub-collection for tokens, components, and documentation.",
    color: "#ec4899",
    link_count: 1,
    public: false,
    parent_id: some(1),
    share_url: "/public/collections/5",
  },
];

const collection = (id) => {
  const found = collections.find((entry) => entry.id === id);
  if (!found) throw new Error(`Unknown preview collection: ${id}`);
  return found;
};

const collectionOptions = (selectedId = null) =>
  collections.map((entry) => ({
    id: entry.id,
    name: entry.name,
    emoji: entry.emoji,
    selected: entry.id === selectedId,
  }));

const link = (input) => {
  const assignedTags = input.tagNames.map(tag);
  const parent = collection(input.collectionId);

  return {
    id: input.id,
    display_title: input.title,
    url: input.url,
    domain: new URL(input.url).hostname.replace(/^www\./, ""),
    description: input.description,
    collection_id: parent.id,
    collection_name: parent.name,
    collection_emoji: parent.emoji,
    collection_href: `/collections/${parent.id}`,
    date: input.date,
    thumb: input.thumb ?? "/static/vite/images/hero.png",
    pinned: input.pinned ?? false,
    tags: assignedTags.map((entry) => ({ ...entry })),
    tags_csv: assignedTags.map((entry) => entry.name).join(", "),
    formats: input.formats ?? [formats.readable, formats.webpage, formats.screenshot],
    highlights: input.highlights ?? [],
  };
};

export const links = [
  link({
    id: 101,
    title: "Designing for the web",
    url: "https://example.com/designing-for-the-web",
    collectionId: 1,
    tagNames: ["design", "frontend"],
    date: "Nov 8, 2026",
    pinned: true,
    description:
      "A practical guide to spacing, rhythm, and hierarchy. The article walks through a small design system, then applies it to a reading interface with a narrow measure and generous line height.",
    highlights: [
      {
        color: "yellow",
        text: "Good interfaces make the next action obvious without making the current one noisy.",
      },
      {
        color: "green",
        text: "Start with the reading column. Every other layout decision follows from it.",
      },
    ],
  }),
  link({
    id: 102,
    title: "Rust ownership without the fear",
    url: "https://example.com/rust-ownership",
    collectionId: 2,
    tagNames: ["rust", "reading"],
    date: "Nov 6, 2026",
    description:
      "A gentle tour of borrows, moves, and lifetimes for people coming from garbage-collected languages. Includes a checklist for refactoring functions that fight the borrow checker.",
  }),
  link({
    id: 103,
    title: "A field guide to CSS container queries",
    url: "https://example.com/css-container-queries",
    collectionId: 1,
    tagNames: ["css", "frontend", "design"],
    date: "Nov 4, 2026",
    description:
      "Container queries let components respond to their own space rather than the viewport. This field guide covers container-type, named containers, and the patterns that scale well in a design system.",
  }),
  link({
    id: 104,
    title: "The quiet art of taking notes",
    url: "https://example.com/notes",
    collectionId: 3,
    tagNames: ["reading", "tools"],
    date: "Nov 2, 2026",
    description:
      "Notes are not an archive; they are a conversation with your future self. A short essay on keeping a lightweight, searchable notebook.",
  }),
  link({
    id: 105,
    title: "Building a small search index",
    url: "https://example.com/search-index",
    collectionId: 2,
    tagNames: ["rust", "tools"],
    date: "Oct 30, 2026",
    description:
      "A from-scratch inverted index in a few hundred lines of Rust, with tokenization, ranking, and a tiny query language.",
  }),
  link({
    id: 106,
    title: "Weeknight tomato soup",
    url: "https://example.com/tomato-soup",
    collectionId: 4,
    tagNames: ["reading"],
    date: "Oct 28, 2026",
    description: "Roasted tomatoes, a little garlic, and a lot of patience. The trick is to blend only half the pot.",
  }),
  link({
    id: 107,
    title: "Design tokens are a contract",
    url: "https://example.com/design-tokens",
    collectionId: 5,
    tagNames: ["design", "css", "frontend"],
    date: "Oct 26, 2026",
    description:
      "Tokens work best when they describe intent rather than raw values. Examples for colour, spacing, and type scales across light and dark themes.",
  }),
  link({
    id: 108,
    title: "Reading a database query plan",
    url: "https://example.com/query-plan",
    collectionId: 2,
    tagNames: ["rust", "reading"],
    date: "Oct 22, 2026",
    description: "What EXPLAIN tells you, what it hides, and how to use it to find the missing index without guessing.",
  }),
];

const linksForCollection = (id) => links.filter((entry) => entry.collection_id === id);
const linksForTag = (name) => links.filter((entry) => entry.tags.some((entryTag) => entryTag.name === name));

export const stats = {
  pages: 128,
  captures: 342,
  collections: collections.length,
  tags: tags.length,
  pinned: 6,
  pendings: 4,
};

export const pendingItems = [
  {
    id: 1,
    url: "https://example.com/a-new-article",
    status_label: "Queued",
    status_class: "badge-info",
    attempts: 0,
    last_error: null,
    date: "2 minutes ago",
    tags: [tag("design")],
    tags_csv: "design",
    is_queued: true,
    is_failed: false,
    retryable: false,
    in_progress: false,
  },
  {
    id: 2,
    url: "https://example.com/docs/getting-started",
    status_label: "Queued",
    status_class: "badge-info",
    attempts: 0,
    last_error: null,
    date: "8 minutes ago",
    tags: [tag("reading"), tag("tools")],
    tags_csv: "reading, tools",
    is_queued: true,
    is_failed: false,
    retryable: false,
    in_progress: false,
  },
  {
    id: 3,
    url: "https://example.com/a-slow-page",
    status_label: "In progress",
    status_class: "badge-warning",
    attempts: 1,
    last_error: null,
    date: "12 minutes ago",
    tags: [tag("frontend")],
    tags_csv: "frontend",
    is_queued: false,
    is_failed: false,
    retryable: false,
    in_progress: true,
  },
  {
    id: 4,
    url: "https://example.com/broken-link",
    status_label: "Failed",
    status_class: "badge-error",
    attempts: 3,
    last_error: "Timed out while fetching the page after 30 seconds.",
    date: "1 hour ago",
    tags: [],
    tags_csv: "",
    is_queued: false,
    is_failed: true,
    retryable: true,
    in_progress: false,
  },
];

export const pendingCounts = {
  queued: pendingItems.filter((entry) => entry.is_queued).length,
  in_progress: pendingItems.filter((entry) => entry.in_progress).length,
  failed: pendingItems.filter((entry) => entry.is_failed).length,
  total: pendingItems.length,
};

export const archivePending = [
  {
    id: 1,
    url: "https://example.com/a-new-article",
    status: "queued",
    created_at: "2 minutes ago",
    tags: "design",
  },
  {
    id: 2,
    url: "https://example.com/docs/getting-started",
    status: "queued",
    created_at: "8 minutes ago",
    tags: "reading, tools",
  },
  {
    id: 3,
    url: "https://example.com/a-slow-page",
    status: "in_progress",
    created_at: "12 minutes ago",
    tags: "frontend",
  },
  {
    id: 4,
    url: "https://example.com/broken-link",
    status: "failed",
    created_at: "1 hour ago",
    tags: "",
  },
];

export const archiveTags = [
  { id: 1, name: "design", pages: 4, pending: 1 },
  { id: 2, name: "rust", pages: 3, pending: 0 },
  { id: 3, name: "frontend", pages: 2, pending: 1 },
  { id: 4, name: "reading", pages: 2, pending: 1 },
  { id: 5, name: "tools", pages: 1, pending: 1 },
  { id: 6, name: "css", pages: 1, pending: 0 },
];

export const archivePages = [
  {
    id: 101,
    display_title: "Designing for the web",
    url: "https://example.com/designing-for-the-web",
    host: "example.com",
    search: "designing for the web https://example.com/designing-for-the-web design frontend",
    pinned: true,
    last_captured: "2 hours ago",
    capture_count: 2,
    version_label: "2 versions",
    tags: [
      { id: 1, name: "design" },
      { id: 3, name: "frontend" },
    ],
    captures: [
      {
        id: 1001,
        version: 2,
        title: "Designing for the web",
        captured_at: "2 hours ago",
        status: "200",
        has_html: true,
        has_screenshot: true,
      },
      {
        id: 1002,
        version: 1,
        title: "Designing for the web (older capture)",
        captured_at: "3 days ago",
        status: "200",
        has_html: true,
        has_screenshot: false,
      },
    ],
  },
  {
    id: 103,
    display_title: "A field guide to CSS container queries",
    url: "https://example.com/css-container-queries",
    host: "example.com",
    search: "a field guide to css container queries https://example.com/css-container-queries css frontend design",
    pinned: false,
    last_captured: "1 day ago",
    capture_count: 1,
    version_label: "1 version",
    tags: [
      { id: 6, name: "css" },
      { id: 3, name: "frontend" },
    ],
    captures: [
      {
        id: 1003,
        version: 1,
        title: "A field guide to CSS container queries",
        captured_at: "1 day ago",
        status: "200",
        has_html: true,
        has_screenshot: true,
      },
    ],
  },
  {
    id: 105,
    display_title: "Building a small search index",
    url: "https://example.com/search-index",
    host: "example.com",
    search: "building a small search index https://example.com/search-index rust tools",
    pinned: false,
    last_captured: "4 days ago",
    capture_count: 3,
    version_label: "3 versions",
    tags: [
      { id: 2, name: "rust" },
      { id: 5, name: "tools" },
    ],
    captures: [
      {
        id: 1004,
        version: 3,
        title: "Building a small search index",
        captured_at: "4 days ago",
        status: "200",
        has_html: true,
        has_screenshot: true,
      },
      {
        id: 1005,
        version: 2,
        title: "Building a small search index",
        captured_at: "10 days ago",
        status: "200",
        has_html: true,
        has_screenshot: false,
      },
      {
        id: 1006,
        version: 1,
        title: "Building a small search index (draft)",
        captured_at: "20 days ago",
        status: "404",
        has_html: false,
        has_screenshot: false,
      },
    ],
  },
];

export const sections = [
  {
    title: "Recently added",
    href: "/links",
    links: links.slice(0, 5),
  },
  {
    title: "🎨 Design",
    href: "/collections/1",
    links: linksForCollection(1),
  },
  {
    title: "🧑‍💻 Development",
    href: "/collections/2",
    links: linksForCollection(2),
  },
  {
    title: "📚 Reading list",
    href: "/collections/3",
    links: linksForCollection(3),
  },
];

export function page(active) {
  return {
    csrf_token: "local",
    nav_collections: collections.map((entry) => ({
      id: entry.id,
      name: entry.name,
      emoji: entry.emoji,
      link_count: entry.link_count,
    })),
    nav_tags: tags.map((entry) => ({
      name: entry.name,
      slug: entry.slug,
      count: entry.count,
    })),
    all_collections: collectionOptions(),
    active,
    unorganized_count: 2,
    pending_count: pendingCounts.queued,
  };
}

export function collectionPage(id, active = "collections") {
  return {
    page: page(active),
    collection: collection(id),
    links: linksForCollection(id),
  };
}

export function tagPage(name) {
  const entry = tag(name);
  return {
    page: page(entry.slug),
    tag: entry,
    links: linksForTag(name),
  };
}

export function linkDetailPage(id, edit = false) {
  const entry = links.find((candidate) => candidate.id === id);
  if (!entry) throw new Error(`Unknown preview link: ${id}`);
  return {
    page: page("links"),
    link: entry,
    collections: collectionOptions(entry.collection_id),
    edit,
  };
}

export function collectionFormPage(editing, id = 1) {
  const entry = collection(id);
  return {
    page: page("collections"),
    editing,
    form_title: editing ? "Edit Collection" : "New Collection",
    submit_label: editing ? "Save changes" : "Create collection",
    form_action: editing ? `/collections/${entry.id}` : "/collections",
    name: editing ? entry.name : "",
    emoji: editing ? entry.emoji : "📁",
    description: editing ? entry.description : "",
    color: editing ? entry.color : "#6d5dfc",
    collections: collectionOptions(editing ? (entry.parent_id.value ?? null) : null),
  };
}

export function pendingPage(status = null) {
  const filtered = status
    ? pendingItems.filter((entry) =>
        status === "queued" ? entry.is_queued : status === "in_progress" ? entry.in_progress : entry.is_failed,
      )
    : pendingItems;

  return {
    page: page("pending"),
    pending: filtered,
    counts: pendingCounts,
    all_active: status === null,
    queued_active: status === "queued",
    in_progress_active: status === "in_progress",
    failed_active: status === "failed",
    notice: status === null ? "2 added, 1 duplicate skipped." : null,
  };
}

export function archivePage() {
  return {
    pendings: archivePending,
    stats,
    pages: archivePages,
    tags: archiveTags,
  };
}

export function notFoundPage() {
  return {
    status: 404,
    title: "Not Found",
    message: "The page you are looking for does not exist.",
  };
}
