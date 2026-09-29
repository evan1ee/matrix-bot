use crate::db::DbPool;
use chrono::Utc;
use serde_json::json;

use crate::events::EventBroadcaster;
use crate::infrastructure::inference::llm::ToolDefinition;
use crate::llm::retrospective::generate_weekly_retrospective;

fn normalize_footprint_category(input: &str) -> &'static str {
    let lower = input.trim().to_lowercase();
    match lower.as_str() {
        "sightseeing" | "scenic" | "attraction" | "landmark" | "tourism" | "景区" | "景点"
        | "名胜" | "古迹" | "观光" => "sightseeing",
        "food" | "dining" | "restaurant" | "cafe" | "coffee" | "bar" | "美食" | "餐饮" | "餐厅"
        | "咖啡" => "food",
        "nature" | "park" | "parks" | "mountain" | "hiking" | "outdoor" | "beach" | "lake"
        | "自然" | "公园" | "山" | "森林" | "湖" | "海滩" | "徒步" => "nature",
        "lodging" | "hotel" | "hostel" | "resort" | "inn" | "bnb" | "住宿" | "酒店" | "宾馆"
        | "民宿" => "lodging",
        "roadtrip" | "road_trip" | "drive" | "camping" | "自驾" | "露营" => "roadtrip",
        "work" | "business" | "conference" | "office" | "出差" | "工作" | "会议" => "work",
        _ => "sightseeing",
    }
}

pub async fn execute_tool(
    services: &crate::application::AppServices,
    pool: &DbPool,
    _broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let settings = services.settings.get_or_default(&acct).await;
    let today_str = Utc::now()
        .with_timezone(&settings.tz())
        .format("%Y-%m-%d")
        .to_string();

    match name {
        "log_life_day" => Some(
            async {
                let mood = args
                    .get("mood")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'mood' is required")?;
                let note = args.get("note").and_then(|v| v.as_str());
                let weather = args.get("weather").and_then(|v| v.as_str());
                let activities: Option<Vec<String>> =
                    args.get("activities").and_then(|v| v.as_array()).map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    });
                let date_str = args
                    .get("date")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| today_str.clone());

                let activities_json = activities
                    .as_ref()
                    .and_then(|acts| serde_json::to_string(acts).ok());

                let existing = services
                    .life
                    .get_day_log(&acct, &date_str)
                    .await
                    .map_err(|e| e.to_string())?;
                if existing.is_some() {
                    return Err(format!(
                        "A log for date {} already exists and cannot be edited. It can only be cleared.",
                        date_str
                    ));
                }

                let log = services
                    .life
                    .upsert_day_log(
                        &acct,
                        &date_str,
                        mood,
                        weather,
                        activities_json.as_deref(),
                        note,
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&log).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_life_profile" => Some(
            async {
                let profile = services
                    .life
                    .get_profile(&acct)
                    .await
                    .map_err(|e| e.to_string())?;
                let stats = services
                    .life
                    .get_stats(&acct, &today_str)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&json!({
                    "profile": profile,
                    "stats": stats,
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_daily_log" => Some(
            async {
                let date_str = args
                    .get("date")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| today_str.clone());

                let day_log = services
                    .life
                    .get_day_log(&acct, &date_str)
                    .await
                    .map_err(|e| e.to_string())?;

                let habits = services
                    .habits
                    .list_with_status(&acct, &date_str)
                    .await
                    .unwrap_or_default();

                serde_json::to_string(&json!({
                    "date": date_str,
                    "day_log": day_log,
                    "habits_status": habits,
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_weekly_retrospective" => Some(
            async {
                let retro = generate_weekly_retrospective(pool, &services.settings, &acct)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&retro).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_my_footprints" => Some(
            async {
                let footprints = services
                    .footprints
                    .list(&acct, None, None, None)
                    .await
                    .map_err(|e| e.to_string())?;
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10) as usize;
                let sliced: Vec<_> = footprints.into_iter().take(limit).collect();
                serde_json::to_string(&sliced).map_err(|e| e.to_string())
            }
            .await,
        ),

        "record_footprint" => Some(
            async {
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'title' is required")?;
                let lat = args
                    .get("latitude")
                    .and_then(|v| v.as_f64())
                    .ok_or("Field 'latitude' is required")?;
                let lon = args
                    .get("longitude")
                    .and_then(|v| v.as_f64())
                    .ok_or("Field 'longitude' is required")?;
                let city = args.get("city").and_then(|v| v.as_str());
                let country = args.get("country").and_then(|v| v.as_str());
                let raw_category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("sightseeing");
                let category = normalize_footprint_category(raw_category);
                let rating = args
                    .get("rating")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, 5);
                let spend_str = args.get("spend_amount").and_then(|v| v.as_str());
                let currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&settings.currency);
                let note = args.get("note").and_then(|v| v.as_str());
                let arrival_date = args
                    .get("arrival_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&today_str);
                let departure_date = args.get("departure_date").and_then(|v| v.as_str());

                let input = crate::application::footprints::CreateFootprintInput {
                    title: title.to_string(),
                    latitude: lat,
                    longitude: lon,
                    country: country.map(|s| s.to_string()),
                    city: city.map(|s| s.to_string()),
                    address: None,
                    arrival_date: arrival_date.to_string(),
                    departure_date: departure_date.map(|s| s.to_string()),
                    spend_amount: spend_str.map(|s| s.to_string()),
                    spend_currency: Some(currency.to_string()),
                    rating: Some(rating),
                    category: Some(category.to_string()),
                    note: note.map(|s| s.to_string()),
                };

                let fp = services
                    .footprints
                    .create(&acct, input)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&fp).map_err(|e| e.to_string())
            }
            .await,
        ),

        "delete_footprint" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                services
                    .footprints
                    .delete(&acct, id)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&json!({
                    "id": id,
                    "deleted": true
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "log_life_day",
            "Log a daily journal entry, mood rating (1-5), headline, and tags in the Life Grid module.",
            json!({
                "type": "object",
                "properties": {
                    "date": {
                        "type": "string",
                        "description": "Date in YYYY-MM-DD format (defaults to today in user's timezone)"
                    },
                    "score": {
                        "type": "integer",
                        "description": "Overall day score (1-5)"
                    },
                    "mood": {
                        "type": "string",
                        "description": "Mood descriptor (e.g. happy, productive, stressed, calm)"
                    },
                    "title": {
                        "type": "string",
                        "description": "Short summary or headline for the day"
                    },
                    "journal": {
                        "type": "string",
                        "description": "Detailed journal entry or reflection for the day"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags (e.g. work, family, vacation)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_daily_log",
            "Fetch the journal entry, mood score, and reflection for a specific date (defaults to today).",
            json!({
                "type": "object",
                "properties": {
                    "date": {
                        "type": "string",
                        "description": "Date in YYYY-MM-DD format (defaults to today)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_life_profile",
            "Fetch user life profile, current age in weeks, total estimated lifespan, and life percentage lived.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_my_footprints",
            "Fetch recorded places and geographic footprints visited by the user.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of footprints to return (default 20, max 100)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "record_footprint",
            "Record a geographic check-in or visited place in the user's travel footprints map.",
            json!({
                "type": "object",
                "properties": {
                    "place_name": {
                        "type": "string",
                        "description": "Name of the place, city, or venue visited (required)"
                    },
                    "latitude": {
                        "type": "number",
                        "description": "Latitude coordinate"
                    },
                    "longitude": {
                        "type": "number",
                        "description": "Longitude coordinate"
                    },
                    "category": {
                        "type": "string",
                        "description": "Optional category (e.g. travel, restaurant, work, nature)"
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional notes or impressions about the visit"
                    },
                    "visited_at": {
                        "type": "string",
                        "description": "Optional visit timestamp in ISO-8601 or YYYY-MM-DD format"
                    }
                },
                "required": ["place_name"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "delete_footprint",
            "Delete a footprint/travel pin by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the footprint to delete (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_weekly_retrospective",
            "Generate an aggregated retrospective summary for a given week covering tasks completed, financial flow, habits, and mood.",
            json!({
                "type": "object",
                "properties": {
                    "week_offset": {
                        "type": "integer",
                        "description": "0 for this week, 1 for last week, etc. (defaults to 0)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
    ]
}
