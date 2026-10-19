use crate::cli::DashboardCommands;
use crate::client::SplunkClient;
use crate::error::Result;
use crate::output::{self, Format};

pub async fn handle_dashboards(
    client: &SplunkClient,
    cmd: &DashboardCommands,
    format: Format,
) -> Result<()> {
    match cmd {
        DashboardCommands::List { app, filter } => {
            let items = client.get_dashboards(app.as_deref(), filter.as_deref()).await?;
            output::print_dashboards(&items, format);
        }
        DashboardCommands::Panels { name, app } => {
            let items = client.get_dashboard_panels(name, app.as_deref()).await?;
            output::print_dashboard_panels(&items, format);
        }
    }
    Ok(())
}
