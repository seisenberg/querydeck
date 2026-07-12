// Capture README screenshots from the vite dev server (mock backend).
// Usage: node scripts/readme-shots.mjs   (expects vite on :1420)
import { chromium } from "playwright";
import { mkdirSync } from "fs";

mkdirSync("docs", { recursive: true });

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1360, height: 850 },
  deviceScaleFactor: 2,
  colorScheme: "dark",
});

await page.goto("http://localhost:1420", { waitUntil: "networkidle" });
await page.waitForSelector(".tree-node", { timeout: 10_000 });

// Expand the active connection so the tree shows schemas + tables.
await page.click('[data-profile-id="demo-pg"]');
await page.waitForSelector("text=customers");
// Consoles under it too (already open by default). Run the active query.
await page.click("button.primary:has-text('Run')");
await page.waitForSelector(".log-row, .rtab, td, .grid-cell", { timeout: 10_000 });
// Wait for the grid to settle (results pane replaces log view).
await page.waitForTimeout(1400);
await page.screenshot({ path: "docs/screenshot-main.png" });
console.log("wrote docs/screenshot-main.png");

// --- connection modal (element shot) ---
await page.click(".sidebar-header .actions button[title='New connection']");
await page.waitForSelector(".modal");
// Show a native analytics engine's form (JDBC/ODBC lead the picker).
await page.click(".driver-picker button:has-text('Trino')");
await page.waitForTimeout(400);
const modal = page.locator(".modal");
await modal.screenshot({ path: "docs/screenshot-connection.png" });
console.log("wrote docs/screenshot-connection.png");
await page.keyboard.press("Escape");
await page.click(".modal-header button", { timeout: 2000 }).catch(() => {});
await page.waitForTimeout(300);

// --- driver manager (element shot) ---
await page.click(".statusbar button:has-text('Drivers')");
await page.waitForSelector(".pkg-row");
const dm = page.locator(".modal");
await dm.screenshot({ path: "docs/screenshot-drivers.png" });
console.log("wrote docs/screenshot-drivers.png");

await browser.close();
