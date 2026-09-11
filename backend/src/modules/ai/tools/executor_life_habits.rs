use chrono::Utc;
use serde_json::json;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::ai::retrospective::generate_weekly_retrospective;
use crate::modules::footprints::repository::FootprintRepository;
use crate::modules::habits::repository::HabitsRepository;
use crate::modules::life::repository::LifeRepository;
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::settings::repository::SettingsRepository;

fn normalize_footprint_category(input: &str) -> &'static str {
    let lower = input.trim().to_lowercase();
    match lower.as_str() {
        "sightseeing" | "scenic" | "attraction" | "landmark" | "tourism" | "景区" | "景点" | "名胜" | "古迹" | "观光" => "sightseeing",
        "food" | "dining" | "restaurant" | "cafe" | "coffee" | "bar" | "美食" | "餐饮" | "餐厅" | "咖啡" => "food",
        "nature" | "park" | "parks" | "mountain" | "hiking" | "outdoor" | "beach" | "lake" | "自然" | "公园" | "山" | "森林" | "湖" | "海滩" | "徒步" => "nature",
        "lodging" | "hotel" | "hostel" | "resort" | "inn" | "bnb" | "住宿" | "酒店" | "宾馆" | "民宿" => "lodging",
        "roadtrip" | "road_trip" | "drive" | "camping" | "自驾" | "露营" => "roadtrip",
        "work" | "business" | "conference" | "office" | "出差" | "工作" | "会议" => "work",
        _ => "sightseeing",
    }
}

pub async fn execute_life_habits_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    _room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
    let today_str = Utc::now().with_timezone(&settings.tz()).format("%Y-%m-%d").to_string();

    match name {
        "get_my_habits" => Some(async {
            let habits = HabitsRepository::find_all_with_status(pool, owner_id, &today_str)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&habits).map_err(|e| e.to_string())
        }.await),

        "log_habit" => Some(async {
            let (habit_id, habit_name) = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                let habits = HabitsRepository::find_all_with_status(pool, owner_id, &today_str)
                    .await
                    .map_err(|e| e.to_string())?;
                let name = habits.into_iter().find(|h| h.habit.id == id).map(|h| h.habit.name).unwrap_or_else(|| format!("Habit #{}", id));
                (id, name)
            } else if let Some(search) = args.get("name_search").or_else(|| args.get("name")).and_then(|v| v.as_str()) {
                let habits = HabitsRepository::find_all_with_status(pool, owner_id, &today_str)
                    .await
                    .map_err(|e| e.to_string())?;
                let matched = habits.into_iter().find(|h| h.habit.name.to_lowercase().contains(&search.to_lowercase()));
                match matched {
                    Some(h) => (h.habit.id, h.habit.name),
                    None => return Err(format!("No habit matching '{}' was found", search)),
                }
            } else {
                return Err("Either 'id', 'name_search', or 'name' is required to log a habit".to_string());
            };

            let completed = HabitsRepository::toggle_log(pool, habit_id, owner_id, &today_str, None)
                .await
                .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::HabitLogToggled {
                    habit_id,
                    owner_id: owner_id.to_string(),
                    log_date: today_str.clone(),
                    completed,
                });
            }

            serde_json::to_string(&json!({
                "habit_id": habit_id,
                "habit_name": habit_name,
                "date": today_str,
                "completed": completed,
                "action": if completed { "marked_completed" } else { "marked_incomplete" }
            })).map_err(|e| e.to_string())
        }.await),

        "create_habit" => Some(async {
            let name_val = args.get("name").and_then(|v| v.as_str()).ok_or("Field 'name' is required")?;
            let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("general");
            let frequency = args.get("frequency").and_then(|v| v.as_str()).unwrap_or("daily");
            let target_days = args.get("target_days_per_week").and_then(|v| v.as_i64()).unwrap_or(7).clamp(1, 7);

            let habit = HabitsRepository::create_habit(
                pool,
                owner_id,
                name_val,
                None,
                frequency,
                category,
                target_days,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::HabitCreated(habit.clone()));
            }

            serde_json::to_string(&habit).map_err(|e| e.to_string())
        }.await),

        "log_life_day" => Some(async {
            let mood = args.get("mood").and_then(|v| v.as_str()).ok_or("Field 'mood' is required")?;
            let note = args.get("note").and_then(|v| v.as_str());
            let weather = args.get("weather").and_then(|v| v.as_str());
            let activities: Option<Vec<String>> = args.get("activities").and_then(|v| v.as_array()).map(|arr| {
                arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
            });
            let date_str = args.get("date").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| today_str.clone());

            let activities_json = activities.as_ref().and_then(|acts| serde_json::to_string(acts).ok());

            let existing = LifeRepository::get_day_log(pool, owner_id, &date_str)
                .await
                .map_err(|e| e.to_string())?;
            if existing.is_some() {
                return Err(format!(
                    "A log for date {} already exists and cannot be edited. It can only be cleared.",
                    date_str
                ));
            }

            let log = LifeRepository::upsert_day_log(
                pool,
                owner_id,
                &date_str,
                mood,
                weather,
                activities_json.as_deref(),
                note,
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::LifeDayLogged(log.clone()));
            }

            serde_json::to_string(&log).map_err(|e| e.to_string())
        }.await),

        "get_life_profile" => Some(async {
            let profile = LifeRepository::get_profile(pool, owner_id).await.map_err(|e| e.to_string())?;
            let stats = LifeRepository::get_stats(pool, owner_id, &today_str).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&json!({
                "profile": profile,
                "stats": stats,
            })).map_err(|e| e.to_string())
        }.await),

        "pay_recurring_bill" => Some(async {
            let bill_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                id
            } else if let Some(search) = args.get("name_search").or_else(|| args.get("name")).and_then(|v| v.as_str()) {
                let bills = RecurringBillRepository::list_by_owner(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                let matched = bills.into_iter().find(|b| b.name.to_lowercase().contains(&search.to_lowercase()));
                match matched {
                    Some(b) => b.id,
                    None => return Err(format!("No recurring bill matching '{}' was found", search)),
                }
            } else {
                return Err("Either 'id', 'name_search', or 'name' is required to pay a recurring bill".to_string());
            };

            let (updated_bill, tx) = RecurringBillRepository::mark_paid(pool, bill_id, owner_id)
                .await
                .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillUpdated(updated_bill.clone()));
                b.send(SystemEvent::TransactionCreated(tx.clone()));
            }

            serde_json::to_string(&json!({
                "bill": updated_bill,
                "transaction_recorded": tx,
            })).map_err(|e| e.to_string())
        }.await),

        "get_my_footprints" => Some(async {
            let footprints = FootprintRepository::list_by_owner(pool, owner_id, None, None, None)
                .await
                .map_err(|e| e.to_string())?;
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10) as usize;
            let sliced: Vec<_> = footprints.into_iter().take(limit).collect();
            serde_json::to_string(&sliced).map_err(|e| e.to_string())
        }.await),

        "record_footprint" => Some(async {
            let title = args.get("title").and_then(|v| v.as_str()).ok_or("Field 'title' is required")?;
            let lat = args.get("latitude").and_then(|v| v.as_f64()).ok_or("Field 'latitude' is required")?;
            let lon = args.get("longitude").and_then(|v| v.as_f64()).ok_or("Field 'longitude' is required")?;
            let city = args.get("city").and_then(|v| v.as_str());
            let country = args.get("country").and_then(|v| v.as_str());
            let raw_category = args.get("category").and_then(|v| v.as_str()).unwrap_or("sightseeing");
            let category = normalize_footprint_category(raw_category);
            let rating = args.get("rating").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 5);
            let spend_str = args.get("spend_amount").and_then(|v| v.as_str()).unwrap_or("0");
            let currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&settings.currency);
            let note = args.get("note").and_then(|v| v.as_str());
            let arrival_date = args.get("arrival_date").and_then(|v| v.as_str()).unwrap_or(&today_str);
            let departure_date = args.get("departure_date").and_then(|v| v.as_str());

            let spend_minor = (spend_str.parse::<f64>().unwrap_or(0.0) * 100.0) as i64;

            let fp = FootprintRepository::create(
                pool,
                owner_id,
                title,
                lat,
                lon,
                country,
                city,
                None,
                arrival_date,
                departure_date,
                spend_minor,
                currency,
                2,
                rating,
                category,
                note,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::FootprintCreated(fp.clone()));
            }

            serde_json::to_string(&fp).map_err(|e| e.to_string())
        }.await),

        "get_weekly_retrospective" => Some(async {
            let retro = generate_weekly_retrospective(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&retro).map_err(|e| e.to_string())
        }.await),

        "save_bookmark" => Some(async {
            let url = args.get("url").and_then(|v| v.as_str()).ok_or("Field 'url' is required")?;
            let custom_title = args.get("title").and_then(|v| v.as_str());
            let tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>().join(",")
            }).unwrap_or_default();

            let meta = crate::modules::bookmarks::metadata::fetch_and_extract_metadata(None, url, custom_title).await;

            let b = crate::modules::bookmarks::repository::BookmarksRepository::create(
                pool,
                owner_id,
                url,
                &meta.title,
                meta.description.as_deref(),
                meta.author.as_deref(),
                meta.image_url.as_deref(),
                &meta.domain,
                meta.reader_content.as_deref(),
                meta.reading_time_minutes,
                &tags,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(bc) = broadcaster {
                bc.send(SystemEvent::BookmarkCreated(b.clone()));
            }

            serde_json::to_string(&b).map_err(|e| e.to_string())
        }.await),

        "get_my_bookmarks" => Some(async {
            let tag = args.get("tag").and_then(|v| v.as_str()).map(|s| s.to_string());
            let fav = args.get("favorite_only").and_then(|v| v.as_bool());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);

            let query = crate::modules::bookmarks::models::BookmarkQuery {
                tag,
                favorite: fav,
                archived: Some(false),
                limit: Some(limit),
                ..Default::default()
            };

            let list = crate::modules::bookmarks::repository::BookmarksRepository::list(pool, owner_id, &query)
                .await
                .map_err(|e| e.to_string())?;

            serde_json::to_string(&list).map_err(|e| e.to_string())
        }.await),

        "delete_habit" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let ok = HabitsRepository::delete_habit(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?;

            if !ok {
                return Err(format!("Habit #{} not found or not owned by user", id));
            }

            if let Some(b) = broadcaster {
                b.send(SystemEvent::HabitDeleted {
                    id,
                    owner_id: owner_id.to_string(),
                });
            }

            serde_json::to_string(&json!({
                "id": id,
                "deleted": true
            })).map_err(|e| e.to_string())
        }.await),

        "delete_footprint" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let ok = FootprintRepository::delete(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?;

            if !ok {
                return Err(format!("Footprint #{} not found or not owned by user", id));
            }

            if let Some(b) = broadcaster {
                b.send(SystemEvent::FootprintDeleted {
                    id,
                    owner_id: owner_id.to_string(),
                });
            }

            serde_json::to_string(&json!({
                "id": id,
                "deleted": true
            })).map_err(|e| e.to_string())
        }.await),

        "get_daily_log" => Some(async {
            let date_str = args.get("date").and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| today_str.clone());

            let day_log = LifeRepository::get_day_log(pool, owner_id, &date_str)
                .await
                .map_err(|e| e.to_string())?;

            let habits = HabitsRepository::find_all_with_status(pool, owner_id, &date_str)
                .await
                .unwrap_or_default();

            serde_json::to_string(&json!({
                "date": date_str,
                "day_log": day_log,
                "habits_status": habits,
            })).map_err(|e| e.to_string())
        }.await),

        _ => None,
    }
}
