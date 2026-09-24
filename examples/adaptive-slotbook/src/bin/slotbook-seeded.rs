//! Deliberately incorrect candidate: anonymous mutations and volatile bookings.
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    milkdrift_slotbook::serve(milkdrift_slotbook::Candidate::SeededFixture).await
}
