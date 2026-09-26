import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router-dom";
import { AppRoot } from "./app/App";
import { initializeTheme } from "./features/profile/theme";

initializeTheme();

const router = createBrowserRouter([{ path: "*", element: <AppRoot /> }]);

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("StatusDeck root element was not found");
}

createRoot(rootElement).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>,
);
