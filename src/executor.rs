use burberry::Executor;
use std::fmt::Debug;
use std::marker::PhantomData;

#[derive(Default)]
pub struct EchoExecutor<T>(PhantomData<T>);

#[async_trait::async_trait]
impl<T: Debug + Send + Sync> Executor<T> for EchoExecutor<T> {
    async fn execute(&self, action: T) -> eyre::Result<()> {
        println!("action: {:?}", action);
        Ok(())
    }
}
