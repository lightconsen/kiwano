// Vitest setup: what every test file gets before it runs.
//
// `jest-dom`'s matchers (`toBeInTheDocument`, `toHaveTextContent`) plus React
// Testing Library's cleanup, which unmounts whatever a test rendered so the
// next one starts from an empty document. Without the cleanup a screen's
// `setInterval` poll (the gateway-status one) keeps ticking after the test
// file's last assertion and React logs "not wrapped in act" from a document
// nobody is looking at.
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  cleanup();
});
