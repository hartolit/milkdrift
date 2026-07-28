# Application crates

Applications are process and presentation boundaries. They own event loops,
environment-specific paths, presentation, attachment to long-lived services, and
process exit behavior. Reusable application semantics belong behind
`application-runtime`; independently stateful subsystems belong in capability
engines rather than individual frontends.

Current application:

- `desktop-slint`: Slint component construction, callback mapping, frame polling,
  and presentation over `application-runtime`.

A future TUI, headless node host, Tauri process, or other native frontend should
reuse E1 rather than rebuild model lifecycle and conversation state. A frontend
may attach to a service whose lifetime outlives that frontend. Lower layers never
import application or UI types.
