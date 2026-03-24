cargo test easy::tests::test_auto_client_connect_and_echo -- --nocapture
cargo test --tests -- --nocapture


cargo run --example auto_switch_dashboard


All factors that can change protocol in current implementation
Switching decision is based on these runtime signals and knobs in AutoSwitchConfig + observed outcomes:

- Probe quality at startup
TCP probe latency
UDP probe latency + ACK success
- Rolling RTT (EMA) per transport
Higher RTT raises score
- Timeout count
Timeouts add penalties to score
- Failure count
IO/protocol failures add penalties
Consecutive failures
If active transport hits threshold, immediate failover pressure
Score margin
Alternate transport must beat active transport by min_score_margin_ms
Anti-flap controls
switch_cooldown
min_dwell_time
And now in dashboard testing mode, these additional injected factors directly influence switching:

Artificial per-transport delay
Artificial periodic per-transport failures