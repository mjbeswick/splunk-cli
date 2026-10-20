use crate::cli::SearchArgs;
use crate::client::SplunkClient;
use crate::error::Result;
use crate::output::{self, Format};

pub async fn handle_search(
    client: &SplunkClient,
    args: &SearchArgs,
    oneshot: bool,
    format: Format,
) -> Result<()> {
    let results = if oneshot {
        client
            .oneshot_search(
                &args.query,
                &args.earliest_time,
                &args.latest_time,
                args.max_results,
            )
            .await?
    } else {
        client
            .async_search(
                &args.query,
                &args.earliest_time,
                &args.latest_time,
                args.max_results,
            )
            .await?
    };

    output::print_search_results(&results, format);
    Ok(())
}

pub async fn handle_alerts(client: &SplunkClient, count: i64, format: Format) -> Result<()> {
    let alerts = client.get_fired_alerts(count).await?;
    output::print_alerts(&alerts, format);
    Ok(())
}

pub async fn handle_indexes(client: &SplunkClient, format: Format) -> Result<()> {
    let indexes = client.get_indexes().await?;
    output::print_indexes(&indexes, format);
    Ok(())
}
