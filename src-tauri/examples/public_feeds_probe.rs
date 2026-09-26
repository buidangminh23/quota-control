//! Fetch every public feed once into a scratch folder and print what came back.
//! `cargo run -p quota-control --example public_feeds_probe -- <folder>`

use quota_control_lib::public_feeds::{FeedName, PublicFeeds};

#[tokio::main]
async fn main() {
    let folder = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "feeds-probe".into());
    let feeds = PublicFeeds::new(folder.into());
    for name in FeedName::ALL {
        let started = std::time::Instant::now();
        let (snapshot, changed) = feeds.refresh(name, false).await;
        println!(
            "{name:?}: {} bytes, changed {changed}, error {:?}, stale {}, {:.1}s",
            snapshot.body.as_ref().map_or(0, String::len),
            snapshot.error,
            snapshot.stale,
            started.elapsed().as_secs_f64()
        );
    }
}
