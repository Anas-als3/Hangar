import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { startEventListeners } from "./store";
import "./index.css";

// SPEC.md §7: both event listeners are registered ONCE at app startup, before anything renders —
// not in a component effect (StrictMode would double-register) and never inside the log panel,
// which would lose every line emitted while it was closed.
startEventListeners();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
