use serde::Serialize;

#[derive(Serialize)]
struct Report {
    ok: bool,
}

#[tokio::main]
async fn main() {
    println!("{}", serde_json::to_string(&Report { ok: true }).unwrap());
}
// pr
