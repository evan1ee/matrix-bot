//! Pure domain types, invariants, and validation rules for Bookmarks.
//!
//! Depends only on std, serde, and thiserror.

use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

pub mod html;
pub use html::ExtractedMetadata;

pub const MAX_URL_LEN: usize = 2048;

/// The user-editable fields of a bookmark, already resolved (an omitted field
/// replaced by its current value), as written by one update.
pub struct BookmarkFields<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub is_favorite: bool,
    pub is_archived: bool,
    pub tags: &'a str,
}
pub const MAX_TITLE_LEN: usize = 500;
pub const MAX_DESCRIPTION_LEN: usize = 2000;
pub const MAX_TAGS_COUNT: usize = 20;
pub const MAX_SINGLE_TAG_LEN: usize = 50;
pub const MAX_TAGS_LEN: usize = 500;

/// A stored bookmark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub owner_id: AccountId,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub domain: String,
    pub reader_content: Option<String>,
    pub reading_time_minutes: i64,
    pub is_favorite: bool,
    pub is_archived: bool,
    pub tags: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Validated input for creating a bookmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBookmark {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub domain: String,
    pub reader_content: Option<String>,
    pub reading_time_minutes: i64,
    pub tags: String,
}

/// Validated input for updating an existing bookmark.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateBookmark {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub is_favorite: Option<bool>,
    pub is_archived: Option<bool>,
}

/// Query filters for listing bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BookmarkQuery {
    pub archived: Option<bool>,
    pub favorite: Option<bool>,
    pub tag: Option<String>,
    pub domain: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Aggregate domain statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainCount {
    pub domain: String,
    pub count: i64,
}

/// Aggregated bookmark statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkStats {
    pub total_count: i64,
    pub unread_count: i64,
    pub archived_count: i64,
    pub favorite_count: i64,
    pub tags: Vec<String>,
    pub top_domains: Vec<DomainCount>,
}

/// Bookmark domain validation and business rule errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BookmarkError {
    #[error("URL cannot be empty")]
    UrlEmpty,
    #[error("URL must begin with http:// or https://")]
    UrlScheme,
    #[error("URL exceeds maximum length of 2048 characters")]
    UrlTooLong,
    #[error("Title exceeds 500 characters")]
    TitleTooLong,
    #[error("Description exceeds 2000 characters")]
    DescriptionTooLong,
    #[error("Cannot specify more than 20 tags")]
    TooManyTags,
    #[error("Tag exceeds 50 characters")]
    TagTooLong,
    #[error("Tags exceed maximum length of 500 characters")]
    TagsTooLong,
    #[error("Bookmark not found")]
    NotFound,
}

/// Validates raw target URL.
pub fn validate_url(raw: &str) -> Result<&str, BookmarkError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BookmarkError::UrlEmpty);
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(BookmarkError::UrlScheme);
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err(BookmarkError::UrlTooLong);
    }
    Ok(trimmed)
}

/// Validates a title string.
pub fn validate_title(title: &str) -> Result<(), BookmarkError> {
    if title.len() > MAX_TITLE_LEN {
        return Err(BookmarkError::TitleTooLong);
    }
    Ok(())
}

/// Validates a description string.
pub fn validate_description(desc: &str) -> Result<(), BookmarkError> {
    if desc.len() > MAX_DESCRIPTION_LEN {
        return Err(BookmarkError::DescriptionTooLong);
    }
    Ok(())
}

/// Validates and normalises a slice of tag strings into a comma-separated string.
pub fn validate_and_normalize_tag_list(tags: &[String]) -> Result<String, BookmarkError> {
    if tags.len() > MAX_TAGS_COUNT {
        return Err(BookmarkError::TooManyTags);
    }
    for tag in tags {
        if tag.len() > MAX_SINGLE_TAG_LEN {
            return Err(BookmarkError::TagTooLong);
        }
    }
    let joined = tags
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(",");

    if joined.len() > MAX_TAGS_LEN {
        return Err(BookmarkError::TagsTooLong);
    }

    Ok(joined)
}

/// Normalises and validates a comma-separated tag string (e.g. from chat `tag1,tag2`).
pub fn validate_and_normalize_tag_str(tags_str: &str) -> Result<String, BookmarkError> {
    let clean = tags_str
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();

    validate_and_normalize_tag_list(&clean)
}

impl CreateBookmark {
    pub fn new(
        url: &str,
        meta: ExtractedMetadata,
        user_title: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<Self, BookmarkError> {
        let valid_url = validate_url(url)?.to_string();

        let title = match user_title {
            Some(t) if !t.trim().is_empty() => {
                validate_title(t)?;
                t.trim().to_string()
            }
            _ => {
                let t = meta.title.trim();
                validate_title(t)?;
                t.to_string()
            }
        };

        if let Some(ref desc) = meta.description {
            validate_description(desc)?;
        }

        let tags_str = match tags {
            Some(t) => validate_and_normalize_tag_list(t)?,
            None => String::new(),
        };

        Ok(Self {
            url: valid_url,
            title,
            description: meta.description,
            author: meta.author,
            image_url: meta.image_url,
            domain: meta.domain,
            reader_content: meta.reader_content,
            reading_time_minutes: meta.reading_time_minutes,
            tags: tags_str,
        })
    }
}

impl UpdateBookmark {
    pub fn validate(&self) -> Result<(), BookmarkError> {
        if let Some(ref title) = self.title {
            validate_title(title)?;
        }
        if let Some(ref desc) = self.description {
            validate_description(desc)?;
        }
        if let Some(ref tags) = self.tags {
            validate_and_normalize_tag_list(tags)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_valid() {
        assert_eq!(
            validate_url("https://example.com").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            validate_url("http://example.com/test?a=1").unwrap(),
            "http://example.com/test?a=1"
        );
    }

    #[test]
    fn test_validate_url_empty_or_bad_scheme() {
        assert_eq!(validate_url("   ").unwrap_err(), BookmarkError::UrlEmpty);
        assert_eq!(
            validate_url("ftp://example.com").unwrap_err(),
            BookmarkError::UrlScheme
        );
        assert_eq!(
            validate_url("example.com").unwrap_err(),
            BookmarkError::UrlScheme
        );
    }

    #[test]
    fn test_validate_url_too_long() {
        let long = format!("https://example.com/{}", "a".repeat(2048));
        assert_eq!(validate_url(&long).unwrap_err(), BookmarkError::UrlTooLong);
    }

    #[test]
    fn test_validate_tags() {
        let tags = vec!["rust".to_string(), " programming ".to_string()];
        let normalized = validate_and_normalize_tag_list(&tags).unwrap();
        assert_eq!(normalized, "rust,programming");

        let too_many = (0..21).map(|i| format!("tag{i}")).collect::<Vec<_>>();
        assert_eq!(
            validate_and_normalize_tag_list(&too_many).unwrap_err(),
            BookmarkError::TooManyTags
        );

        let long_tag = vec!["a".repeat(51)];
        assert_eq!(
            validate_and_normalize_tag_list(&long_tag).unwrap_err(),
            BookmarkError::TagTooLong
        );
    }

    #[test]
    fn test_validate_tag_str() {
        let tag_str = " tech, news ,  , rust ";
        let res = validate_and_normalize_tag_str(tag_str).unwrap();
        assert_eq!(res, "tech,news,rust");
    }

    #[test]
    fn test_create_bookmark_validation() {
        let meta = ExtractedMetadata {
            title: "Default Title".to_string(),
            domain: "example.com".to_string(),
            ..Default::default()
        };

        let cb = CreateBookmark::new("https://example.com", meta, None, None).unwrap();
        assert_eq!(cb.url, "https://example.com");
        assert_eq!(cb.title, "Default Title");
        assert_eq!(cb.tags, "");
    }
}
