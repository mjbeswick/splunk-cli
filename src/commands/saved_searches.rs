use crate::cli::SavedSearchCommands;
use crate::client::SplunkClient;
use crate::error::Result;
use crate::output::{self, Format};

pub async fn handle_saved_searches(
    client: &SplunkClient,
    cmd: &SavedSearchCommands,
    format: Format,
) -> Result<()> {
    match cmd {
        SavedSearchCommands::List { app, filter } => {
            let items = client
                .list_saved_searches(app.as_deref(), filter.as_deref())
                .await?;
            output::print_saved_searches(&items, format);
        }
        SavedSearchCommands::Run {
            name,
            app,
            earliest_time,
            latest_time,
        } => {
            let results = client
                .run_saved_search(
                    name,
                    app.as_deref(),
                    earliest_time.as_deref(),
                    latest_time.as_deref(),
                )
                .await?;
            output::print_search_results(&results, format);
        }
    }
    Ok(())
}
