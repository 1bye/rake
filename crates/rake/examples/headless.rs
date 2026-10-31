//! A worker using the same commands/tasks/quit runtime without a GPUI window.
use rake::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let collector = Collector::default();
    let services = Services::new(
        HttpClient::with_collector(HttpConfig::new(), collector.clone())?,
        collector,
        ServiceRegistry::new(),
    );
    let runtime = Runtime::new(RouterConfig::default(), services);
    let context = runtime.context();
    let work = context.tasks().spawn("worker", async {
        rake::tasks::sleep(std::time::Duration::from_millis(10)).await;
        Ok(())
    })?;

    futures::executor::block_on(async {
        let workflow = async {
            work.wait().await?;
            context.quit(QuitOptions::default()).wait().await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (driver, workflow) = futures::join!(runtime.run(), workflow);
        driver?;
        workflow
    })
}
