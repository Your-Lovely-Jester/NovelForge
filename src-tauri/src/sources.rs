//! Declarative source-adapter registry.
//!
//! Every supported host is described by one [`SourceProfile`]. The rest of the
//! app asks the registry instead of hardcoding host string checks, so adding a
//! source means adding a profile entry (plus a parser only when the site does
//! not fit the generic walker).
//!
//! Support is evidence-based. A source is not promoted to [`SupportLevel::Verified`]
//! until it has passed the full live matrix: search, complete chapter index,
//! a 1–3 chapter export, and a truncated-TOC failure. Until then the profile
//! records the strongest thing that was actually proven.

use serde::Serialize;

/// How much of the pipeline a source has actually proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupportLevel {
    /// Search, full index, 1–3 chapter export and a truncated-TOC failure test
    /// have all passed live.
    Verified,
    /// A live run proved the chapter index complete; the download matrix is
    /// not done yet.
    IndexVerified,
    /// Metadata can be read, but the chapter index has not been proven complete.
    MetadataOnly,
    /// Only transport reachability is known.
    Unverified,
}

impl SupportLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::IndexVerified => "index verified",
            Self::MetadataOnly => "metadata only",
            Self::Unverified => "unverified",
        }
    }

    /// Only a verified source may be advertised as fully supported.
    pub fn is_download_ready(self) -> bool {
        matches!(self, Self::Verified)
    }

    /// The chapter index itself has been proven complete live, even if the
    /// download matrix has not. Enough to trust a source as a title donor.
    pub fn is_index_proven(self) -> bool {
        matches!(self, Self::Verified | Self::IndexVerified)
    }
}

/// Which parser family the source needs for metadata and the chapter index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParserKind {
    /// NovelFire-style detail page: `/book/<slug>` plus `/book/<slug>/chapters?page=N`.
    NovelFire,
    /// NovelPhoenix-style page with a chapter list and numbered TOC pages.
    NovelPhoenix,
    /// NovelFull detail page: `h1.tit`, `/author/` links, `#list-chapter` counts.
    NovelFull,
    /// Generic walker: OpenGraph/JSON-LD metadata plus any pagination style.
    Generic,
}

/// How an author name is spelled inside an author-page URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorSlug {
    /// Lower-case words joined by `-`: `Gu Zhen Ren` → `gu-zhen-ren`.
    Hyphen,
    /// Original casing, words joined by `+`: `Gu Zhen Ren` → `Gu+Zhen+Ren`.
    Plus,
}

impl AuthorSlug {
    /// Builds the path segment that the site uses for an author page. Only
    /// alphanumerics survive, so the result can never escape the URL path.
    pub fn build(self, author: &str) -> String {
        let words: Vec<String> = author
            .split_whitespace()
            .map(|word| {
                word.chars()
                    .filter(|character| {
                        character.is_alphanumeric() || matches!(character, '-' | '_')
                    })
                    .collect::<String>()
            })
            .filter(|word| !word.is_empty())
            .map(|word| match self {
                Self::Hyphen => word.to_lowercase(),
                Self::Plus => word,
            })
            .collect();
        match self {
            Self::Hyphen => words.join("-"),
            Self::Plus => words.join("+"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Source search through lncrawl.
    pub search: bool,
    /// App-level metadata fetch.
    pub metadata: bool,
    /// App-level chapter-index fetch.
    pub index: bool,
    /// Chapter content download through lncrawl.
    pub chapters: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    pub id: &'static str,
    pub display_name: &'static str,
    /// Bare host suffixes. A host matches itself and any subdomain.
    pub hosts: &'static [&'static str],
    pub language: &'static str,
    /// Original work languages, e.g. `["ko"]` for a Korean novel in English.
    pub origin_languages: &'static [&'static str],
    pub support: SupportLevel,
    pub parser: ParserKind,
    pub capabilities: Capabilities,
    /// Base URL used for lncrawl search, when search is supported.
    pub search_url: Option<&'static str>,
    /// Author-page URL template containing `{author}`. Author pages list every
    /// novel credited to that author, which a site-wide title search does not.
    pub author_page: Option<&'static str>,
    /// How [`author_page`](Self::author_page) spells the author name.
    pub author_slug: AuthorSlug,
    /// Date the full live matrix passed. Required before `verified`.
    pub live_matrix: Option<&'static str>,
    /// Known catalogue or copyright caveat shown next to the source.
    pub availability_note: Option<&'static str>,
    /// Extra selectors removed from chapter XHTML for this source. Tokens are
    /// `tag`, `.class`, `#id` or `[attr=value]`. Common navigation/ad selectors
    /// are always applied; this list adds source-specific chrome.
    pub chapter_chrome: &'static [&'static str],
    /// What was actually observed, with a date. Never a promise.
    pub evidence: &'static str,
}

impl SourceProfile {
    /// Structural validation. No network access, so the registry can be tested
    /// and shown in the UI before any request is made.
    #[cfg(test)]
    pub fn validate(&self) -> Result<(), String> {
        let mut problems = Vec::new();
        if self.id.trim().is_empty() {
            problems.push("empty id");
        }
        if self.display_name.trim().is_empty() {
            problems.push("empty display name");
        }
        if self.language.trim().is_empty() {
            problems.push("empty language");
        }
        if self.evidence.trim().is_empty() {
            problems.push("empty evidence note");
        }
        if self.capabilities.search && self.search_url.is_none() {
            problems.push("search capability without a search URL");
        }
        if let Some(template) = self.author_page {
            if !template.contains("{author}") {
                problems.push("author page template without {author}");
            }
            if !template.starts_with("https://") {
                problems.push("author page template must use https");
            }
        }
        if self.support.is_download_ready() && self.live_matrix.is_none() {
            problems.push("download-ready profile without a live-matrix date");
        }
        for host in self.hosts {
            if host.trim().is_empty() || host.contains("://") || host.contains('/') {
                problems.push("hosts must be bare host names");
            }
        }
        for token in self.chapter_chrome {
            let token = token.trim();
            let valid = token.starts_with('.')
                || token.starts_with('#')
                || (token.starts_with('[') && token.ends_with(']'))
                || token
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-');
            if token.is_empty() || !valid {
                problems.push("chapter_chrome tokens must be tag, .class, #id or [attr=value]");
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!("{}: {}", self.id, problems.join(", ")))
        }
    }
}

const fn caps(search: bool, index: bool, chapters: bool) -> Capabilities {
    Capabilities {
        search,
        metadata: true,
        index,
        chapters,
    }
}

const GENERIC_PROFILE: SourceProfile = SourceProfile {
    id: "generic",
    display_name: "Generic source",
    hosts: &[],
    language: "en",
    origin_languages: &[],
    support: SupportLevel::Unverified,
    parser: ParserKind::Generic,
    capabilities: Capabilities {
        search: false,
        metadata: true,
        index: true,
        chapters: true,
    },
    search_url: None,
    author_page: None,
    author_slug: AuthorSlug::Hyphen,
    live_matrix: None,
    availability_note: None,
    chapter_chrome: &[],
    evidence: "Fallback only. The generic walker must prove completeness before an index is accepted.",
};

const PROFILES: &[SourceProfile] = &[
    SourceProfile {
        id: "novelfire",
        display_name: "NovelFire",
        hosts: &["novelfire.net"],
        language: "en",
        origin_languages: &["zh", "ja"],
        support: SupportLevel::IndexVerified,
        parser: ParserKind::NovelFire,
        capabilities: caps(true, true, true),
        search_url: Some("https://novelfire.net/"),
        author_page: Some("https://novelfire.net/author/{author}"),
        author_slug: AuthorSlug::Hyphen,
        live_matrix: None,
        availability_note: Some(
            "NovelFire has removed some Korean-origin novels for copyright reasons; its catalogue differs from other English aggregators.",
        ),
        chapter_chrome: &["#chapter-nav", ".chapter-nav", ".adsbygoogle"],
        evidence: "Sequential 900 ms TOC pacing completed a live index; a 6-way burst produced 429. Author page /author/<slug> listed 4/4 Gu Zhen Ren novels (2026-11-16).",
    },
    SourceProfile {
        id: "novelphoenix",
        display_name: "NovelPhoenix",
        hosts: &["novelphoenix.net"],
        language: "en",
        origin_languages: &["ko", "zh", "ja"],
        support: SupportLevel::Verified,
        parser: ParserKind::NovelPhoenix,
        capabilities: caps(true, true, true),
        search_url: Some("https://novelphoenix.net/"),
        author_page: Some("https://novelphoenix.net/author/{author}"),
        author_slug: AuthorSlug::Plus,
        live_matrix: Some("2026-11-15"),
        availability_note: None,
        chapter_chrome: &[".chapter-nav"],
        evidence: "Full live matrix: search, complete index, 1–2/3–4 chapter exports, selection-limited retry, update 112–113 with 1–111 preserved. Author page /author/<name> listed 2/2 Gu Zhen Ren novels (2026-11-16).",
    },
    SourceProfile {
        id: "novelfull",
        display_name: "NovelFull",
        hosts: &["novelfull.net"],
        language: "en",
        origin_languages: &["zh", "ko"],
        support: SupportLevel::Verified,
        parser: ParserKind::NovelFull,
        capabilities: caps(false, true, true),
        search_url: None,
        author_page: None,
        author_slug: AuthorSlug::Hyphen,
        live_matrix: Some("2026-11-15"),
        availability_note: None,
        chapter_chrome: &["#chapter-nav", ".chapter-nav"],
        evidence: "2,334/2,334 chapters from 59 TOC pages over curl at 1/1 concurrency; 1–3 download audited 3/3 with cover, metadata.json, chapter XHTML and a verified EPUB (2m 04s).",
    },
    SourceProfile {
        id: "freewebnovel",
        display_name: "FreeWebNovel",
        hosts: &["freewebnovel.com"],
        language: "en",
        origin_languages: &["zh", "ko", "ja"],
        support: SupportLevel::Verified,
        parser: ParserKind::Generic,
        capabilities: caps(false, true, true),
        search_url: None,
        author_page: None,
        author_slug: AuthorSlug::Hyphen,
        live_matrix: Some("2026-11-15"),
        availability_note: None,
        chapter_chrome: &[".chapter-nav"],
        evidence: "Full live matrix 2026-11-15: 2,334/2,334 chapters from 59 TOC pages in 49 s; 1–3 download 3/3 with cover, metadata.json and a verified EPUB; truncated TOC rejected.",
    },
    SourceProfile {
        id: "divinedaolibrary",
        display_name: "Divine Dao Library",
        hosts: &["divinedaolibrary.com"],
        language: "en",
        origin_languages: &["zh"],
        support: SupportLevel::Verified,
        parser: ParserKind::Generic,
        capabilities: caps(false, true, true),
        search_url: None,
        author_page: None,
        author_slug: AuthorSlug::Hyphen,
        live_matrix: Some("2026-11-15"),
        availability_note: None,
        chapter_chrome: &[".sharedaddy", ".jp-relatedposts", ".post-navigation"],
        evidence: "Full live matrix 2026-11-15: 1,229/1,229 chapters from the single story page; 1–3 download audited 3/3 with cover, metadata.json and a verified 86,212-byte EPUB in 12.3 s.",
    },
    SourceProfile {
        id: "novelping",
        display_name: "NovelPing",
        hosts: &["novelping.com"],
        language: "en",
        origin_languages: &["ko", "zh", "ja"],
        support: SupportLevel::Verified,
        parser: ParserKind::Generic,
        capabilities: caps(false, true, true),
        search_url: None,
        author_page: None,
        author_slug: AuthorSlug::Hyphen,
        live_matrix: Some("2026-11-15"),
        availability_note: None,
        chapter_chrome: &[".chapter-nav"],
        evidence: "Full live matrix 2026-11-15: 53/53 chapters from the AJAX archive; 1–3 download audited 3/3 with cover, metadata.json and a verified 50,960-byte EPUB in 9.5 s.",
    },
    SourceProfile {
        id: "webnovel",
        display_name: "Webnovel",
        hosts: &["webnovel.com"],
        language: "en",
        origin_languages: &["zh", "ko"],
        support: SupportLevel::MetadataOnly,
        parser: ParserKind::Generic,
        capabilities: caps(false, false, true),
        search_url: None,
        author_page: None,
        author_slug: AuthorSlug::Hyphen,
        live_matrix: None,
        availability_note: None,
        chapter_chrome: &[],
        evidence: "Metadata works through the curl fallback (reqwest gets Cloudflare 403), but the catalog exposes 877 rows and only 832 links (45 locked), so a complete index cannot be proven; jobs fail after download.",
    },
    SourceProfile {
        id: "novelbin",
        display_name: "NovelBin",
        hosts: &["novelbin.com"],
        language: "en",
        origin_languages: &["zh", "ko"],
        support: SupportLevel::Unverified,
        parser: ParserKind::Generic,
        capabilities: caps(false, true, true),
        search_url: None,
        author_page: None,
        author_slug: AuthorSlug::Hyphen,
        live_matrix: None,
        availability_note: None,
        chapter_chrome: &[],
        evidence: "Cloudflare-protected clone; conservative transport defaults, not yet live-verified.",
    },
];

/// Every concrete source profile, in preference order. Only the registry
/// tests need the complete slice; release code looks profiles up by host.
#[cfg(test)]
pub fn all() -> &'static [SourceProfile] {
    PROFILES
}

fn host_matches(host: &str, suffix: &str) -> bool {
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

/// Resolves a URL/host to its profile, falling back to the generic profile.
pub fn for_host(host: &str) -> &'static SourceProfile {
    let host = host
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or(host)
        .split(':')
        .next()
        .unwrap_or(host)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    PROFILES
        .iter()
        .find(|profile| {
            profile
                .hosts
                .iter()
                .any(|suffix| host_matches(&host, suffix))
        })
        .unwrap_or(&GENERIC_PROFILE)
}

/// Search-capable sources in registry order.
pub fn search_sources() -> Vec<&'static SourceProfile> {
    PROFILES
        .iter()
        .filter(|profile| profile.capabilities.search)
        .collect()
}

/// Preference rank for resolver tie-breaking; unknown sources sort last.
///
/// This is deliberately not registry order. NovelPhoenix answers a chapter in
/// a single request (measured 2026-11-16: 0.9–1.4 s latency, 20/20 chapters and
/// its 59-page TOC in 56 s total), while NovelFire sends the same chapter
/// through a 302 → JS-stub → mirror chain (7.6 s/chapter, 342 s for the same 20
/// chapters). When two hosts verify the same title, the cheaper host wins.
///
/// NovelFire is last because its three-hop chain is measured. The sources above
/// it are single-host adapters with no observed redirect chain; they are not
/// claimed to be faster than each other, only cheaper than the measured chain.
/// Sources absent from this list keep registry order behind the listed ones, so
/// a new profile cannot silently outrank a measured one.
const PREFERENCE: &[&str] = &[
    "novelphoenix",
    "novelfull",
    "freewebnovel",
    "divinedaolibrary",
    "novelping",
    "novelfire",
];

pub fn rank_for_host(host: &str) -> u8 {
    let profile = for_host(host);
    // The generic fallback has no host of its own; anything routed to it is
    // unknown and must sort last.
    if profile.id == "generic" {
        return u8::MAX;
    }
    if let Some(position) = PREFERENCE.iter().position(|id| *id == profile.id) {
        return position as u8;
    }
    // Unlisted known sources follow the listed ones, preserving registry order.
    let offset = PREFERENCE.len() as u8;
    PROFILES
        .iter()
        .position(|candidate| candidate.id == profile.id)
        .map(|position| offset.saturating_add(position as u8))
        .unwrap_or(u8::MAX)
}

/// Compact rows for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub id: &'static str,
    pub display_name: &'static str,
    pub hosts: Vec<String>,
    pub language: &'static str,
    pub origin_languages: Vec<String>,
    /// Known catalogue or copyright caveat shown next to the source.
    pub availability_note: Option<&'static str>,
    pub support: SupportLevel,
    pub support_label: &'static str,
    pub download_ready: bool,
    pub search: bool,
    pub index: bool,
    pub evidence: &'static str,
}

pub fn summaries() -> Vec<SourceSummary> {
    PROFILES
        .iter()
        .map(|profile| SourceSummary {
            id: profile.id,
            display_name: profile.display_name,
            hosts: profile
                .hosts
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
            language: profile.language,
            origin_languages: profile
                .origin_languages
                .iter()
                .map(|code| (*code).to_string())
                .collect(),
            availability_note: profile.availability_note,
            support: profile.support,
            support_label: profile.support.label(),
            download_ready: profile.support.is_download_ready(),
            search: profile.capabilities.search,
            index: profile.capabilities.index,
            evidence: profile.evidence,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_profile_validates_offline() {
        for profile in all() {
            profile
                .validate()
                .unwrap_or_else(|error| panic!("invalid profile: {error}"));
        }
        GENERIC_PROFILE.validate().expect("generic profile");
    }

    #[test]
    fn ids_and_hosts_are_unique() {
        let mut ids = HashSet::new();
        let mut hosts = HashSet::new();
        for profile in all() {
            assert!(ids.insert(profile.id), "duplicate id {}", profile.id);
            for host in profile.hosts {
                assert!(hosts.insert(*host), "duplicate host {host}");
            }
        }
    }

    #[test]
    fn host_matching_handles_www_subdomains_and_ports() {
        assert_eq!(for_host("novelfull.net").id, "novelfull");
        assert_eq!(for_host("www.novelfull.net").id, "novelfull");
        assert_eq!(for_host("https://novelfull.net:8443/book").id, "novelfull");
        assert_eq!(for_host("NovelPhoenix.NET").id, "novelphoenix");
        assert_eq!(for_host("freewebnovel.com").id, "freewebnovel");
        assert_eq!(
            for_host("https://www.freewebnovel.com/novel/x").id,
            "freewebnovel"
        );
        assert_eq!(for_host("unknown.example").id, "generic");
        assert_eq!(for_host("").id, "generic");
        assert!(!for_host("unknown.example").support.is_download_ready());
    }

    #[test]
    fn generic_fallback_never_claims_completeness_support() {
        assert_eq!(GENERIC_PROFILE.support, SupportLevel::Unverified);
        assert_eq!(GENERIC_PROFILE.id, "generic");
    }

    #[test]
    fn only_verified_sources_are_download_ready() {
        let ready: Vec<&str> = all()
            .iter()
            .filter(|profile| profile.support.is_download_ready())
            .map(|profile| profile.id)
            .collect();
        assert_eq!(
            ready,
            vec![
                "novelphoenix",
                "novelfull",
                "freewebnovel",
                "divinedaolibrary",
                "novelping"
            ]
        );
        for profile in all()
            .iter()
            .filter(|profile| profile.support.is_download_ready())
        {
            assert!(
                profile.live_matrix.is_some(),
                "{} is download-ready without a live-matrix date",
                profile.id
            );
        }
    }

    #[test]
    fn author_pages_are_declared_with_a_slug_style() {
        let novelfire = for_host("www.novelfire.net");
        assert_eq!(
            novelfire.author_page,
            Some("https://novelfire.net/author/{author}")
        );
        assert_eq!(novelfire.author_slug.build("Gu Zhen Ren"), "gu-zhen-ren");

        let novelphoenix = for_host("novelphoenix.net");
        assert_eq!(
            novelphoenix.author_page,
            Some("https://novelphoenix.net/author/{author}")
        );
        assert_eq!(novelphoenix.author_slug.build("Gu Zhen Ren"), "Gu+Zhen+Ren");

        // A profile without an author page must not advertise one.
        assert!(for_host("novelfull.net").author_page.is_none());
        assert!(for_host("unknown.example").author_page.is_none());
    }

    #[test]
    fn author_slug_filters_path_characters() {
        assert_eq!(AuthorSlug::Hyphen.build("  Gu/Zhen Ren?  "), "guzhen-ren");
        assert_eq!(AuthorSlug::Plus.build("蛊真人"), "蛊真人");
        assert_eq!(AuthorSlug::Hyphen.build(""), "");
    }

    #[test]
    fn novelfire_does_not_claim_korean_origin_support() {
        // NovelFire has removed Korean-origin novels for copyright reasons, so
        // the registry must not advertise it as a Korean-translation source.
        let profile = for_host("novelfire.net");
        assert!(!profile.origin_languages.contains(&"ko"));
        assert!(profile.availability_note.is_some());
        assert!(profile.origin_languages.contains(&"zh"));
    }

    #[test]
    fn every_profile_documents_evidence() {
        for profile in all() {
            assert!(
                !profile.evidence.trim().is_empty(),
                "{} has no evidence note",
                profile.id
            );
            if let Some(matrix) = profile.live_matrix {
                assert!(
                    matrix.len() >= 10 && matrix.contains('-'),
                    "{} has an unreadable live-matrix date: {matrix:?}",
                    profile.id
                );
            }
            if profile.support.is_download_ready() {
                assert!(
                    profile.evidence.len() >= 40,
                    "{} is download-ready but its evidence is too thin: {:?}",
                    profile.id,
                    profile.evidence
                );
            }
        }
    }

    #[test]
    fn search_sources_keep_registry_order() {
        // UI/search order stays registry order; only tie-breaking is re-ranked.
        let ids: Vec<&str> = search_sources().iter().map(|profile| profile.id).collect();
        assert_eq!(ids, vec!["novelfire", "novelphoenix"]);
    }

    #[test]
    fn measured_cheaper_source_wins_the_tie_break() {
        // Measured 2026-11-16: NovelPhoenix serves a chapter in one request
        // (0.9–1.4 s), NovelFire needs a 302 → JS stub → mirror chain
        // (7.6 s/chapter). Both carry the same title, so the cheaper host is
        // preferred when confidence ties.
        assert_eq!(rank_for_host("novelphoenix.net"), 0);
        assert!(rank_for_host("novelphoenix.net") < rank_for_host("novelfire.net"));

        // Every known host keeps a finite rank, so a tie between two real
        // sources is always deterministic.
        for profile in all().iter().filter(|profile| !profile.hosts.is_empty()) {
            assert!(
                rank_for_host(profile.hosts[0]) < u8::MAX,
                "{} has no preference rank",
                profile.id
            );
        }

        assert_eq!(rank_for_host("unknown.example"), u8::MAX);
    }
}
