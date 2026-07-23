use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use rusqlite::Connection;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tower_http::services::ServeDir;

use solana_tracker::db;

const DB_PATH: &str = "transfers.db";

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
}

#[derive(Deserialize)]
struct LimitParams {
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct HoursParams {
    hours: Option<u32>,
}

#[derive(Deserialize)]
struct SearchParams {
    address: String,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct WatchlistBody {
    address: String,
    label: Option<String>,
}

fn db_error(e: rusqlite::Error) -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}"))
}

async fn stats(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::stats(&conn) {
        Ok(s) => Json(s).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn recent_transfers(
    State(state): State<AppState>,
    Query(params): Query<LimitParams>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::recent_transfers(&conn, params.limit.unwrap_or(50)) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn top_transfers(
    State(state): State<AppState>,
    Query(params): Query<LimitParams>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::top_transfers(&conn, params.limit.unwrap_or(10)) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn timeseries(
    State(state): State<AppState>,
    Query(params): Query<HoursParams>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::hourly_volume(&conn, params.hours.unwrap_or(24)) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn search_transfers(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::search_transfers(&conn, &params.address, params.limit.unwrap_or(50)) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn watchlist_activity(
    State(state): State<AppState>,
    Query(params): Query<LimitParams>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::watchlist_activity(&conn, params.limit.unwrap_or(50)) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn program_breakdown(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::program_breakdown(&conn) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn size_distribution(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::size_distribution(&conn) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn top_addresses(
    State(state): State<AppState>,
    Query(params): Query<LimitParams>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::top_addresses(&conn, params.limit.unwrap_or(10)) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn hour_of_day_activity(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::activity_by_hour_of_day(&conn) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn list_watchlist(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::list_watchlist(&conn) {
        Ok(t) => Json(t).into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn add_watchlist(
    State(state): State<AppState>,
    Json(body): Json<WatchlistBody>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::add_watchlist(&conn, &body.address, body.label.as_deref()) {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

async fn remove_watchlist(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    match db::remove_watchlist(&conn, &address) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => db_error(e).into_response(),
    }
}

#[tokio::main]
async fn main() {
    let conn = Connection::open(DB_PATH).expect("failed to open sqlite db");
    db::init_db(&conn).expect("failed to init schema");
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
    };

    let app = Router::new()
        .route("/api/stats", get(stats))
        .route("/api/transfers/recent", get(recent_transfers))
        .route("/api/transfers/top", get(top_transfers))
        .route("/api/transfers/search", get(search_transfers))
        .route("/api/timeseries", get(timeseries))
        .route("/api/breakdown/program", get(program_breakdown))
        .route("/api/breakdown/size", get(size_distribution))
        .route("/api/leaderboard", get(top_addresses))
        .route("/api/heatmap", get(hour_of_day_activity))
        .route("/api/watchlist", get(list_watchlist).post(add_watchlist))
        .route(
            "/api/watchlist/:address",
            axum::routing::delete(remove_watchlist),
        )
        .route("/api/watchlist/activity", get(watchlist_activity))
        .fallback_service(ServeDir::new("static"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind to 127.0.0.1:3000");

    println!("Dashboard running at http://127.0.0.1:3000");
    axum::serve(listener, app).await.expect("server error");
}
