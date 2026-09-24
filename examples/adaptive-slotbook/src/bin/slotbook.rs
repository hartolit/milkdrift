//! Corrected immutable candidate for the finite Slotbook qualification.
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    milkdrift_slotbook::serve(milkdrift_slotbook::Candidate::Corrected).await
}
