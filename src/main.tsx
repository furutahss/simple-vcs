import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { api } from "./api";
import "./styles.css";
createRoot(document.getElementById("root")!).render(<StrictMode><App client={api} /></StrictMode>);
