// Must be first: configures Monaco's web-worker before any monaco-editor
// module is evaluated. Import order in static ESM is depth-first, so placing
// this import before App (which transitively loads monaco-editor) ensures the
// MonacoEnvironment is in place before Monaco's own init code runs.
import "./monaco-env";

import "./styles/global.css";
import { initApp } from "./App";

initApp().catch(console.error);
