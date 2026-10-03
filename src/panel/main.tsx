import React from "react";
import ReactDOM from "react-dom/client";
import "../theme.css";
import { SidePanelApp } from "./SidePanelApp";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <SidePanelApp />
  </React.StrictMode>,
);
