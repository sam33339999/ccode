use ccode_application::queries::health;

pub async fn run() -> anyhow::Result<()> {
    let res = health::execute();
    println!("status: {}", res.status);
    Ok(())
}
