use axum::{
    extract::{Json, Query, State},
    response::{
        sse::{Event, Sse},
        Html,
    },
    routing::{get, post},
    Router,
};
use axum_sse_manager::SseManager;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum StreamTag {
    UserId(i32),
    IsAdmin,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "_type")]
enum EventMessage {
    User(UserMessage),
    Admin(SimpleMessage),
    Broadcast(SimpleMessage),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UserMessage {
    user_id: i32,
    message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SimpleMessage {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ConnectionParams {
    user_id: Option<i32>,
    is_admin: bool,
}

#[tokio::main]
async fn main() {
    let sse_sessions = SseManager::new();
    let app = Router::new()
        .route("/", get(index))
        .route("/send", post(send))
        .route("/events", get(events))
        .with_state(sse_sessions);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn index() -> Html<&'static str> {
    Html(include_str!("users.html"))
}

/// Handler for browser to receive SSE events
async fn events(
    State(sessions): State<SseManager<EventMessage, StreamTag>>,
    Query(ConnectionParams { user_id, is_admin }): Query<ConnectionParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut tags = Vec::new();
    if let Some(id) = user_id {
        tags.push(StreamTag::UserId(id));
    }
    if is_admin {
        tags.push(StreamTag::IsAdmin);
    }
    let stream = sessions.create_stream(tags).await;
    Sse::new(stream)
}

async fn send(
    State(sse): State<SseManager<EventMessage, StreamTag>>,
    Json(message): Json<EventMessage>,
) {
    use EventMessage::*;
    match message {
        User(data) => {
            sse.send_by_tag(&StreamTag::UserId(data.user_id), User(data))
                .await
        }
        Admin(data) => sse.send_by_tag(&StreamTag::IsAdmin, Admin(data)).await,
        Broadcast(data) => sse.broadcast(Broadcast(data)).await,
    }
}
