#!/usr/bin/env node
// Requires Playwright and its Chromium browser: npm install playwright && npx playwright install chromium
// Run from any directory: node scripts/export_keyboard_layout.cjs
const { chromium } = require('playwright');
const { mkdir } = require('node:fs/promises');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const root = path.resolve(__dirname, '..');
const names = ['shared', 'organize', 'easy-delete', 'fix-rotation', 'pick-best', 'cull'];

async function main() {
  const output = path.join(root, 'assets', 'keyboard-shortcuts');
  await mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_EXECUTABLE_PATH });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 2 });
    await page.goto(pathToFileURL(path.join(root, 'assets', 'keyboard-layout.html')).href);
    await page.addStyleTag({ content: `
      * { font-family: Arial, sans-serif; }
      .bodyStyle { width: 1157px; }
      #KeyboardTable > li { width: 1157px; padding: 24px; background: white; display: flow-root; }
      #KeyboardTable h3 { float: none !important; margin: 0 0 20px; font-size: 22px; }
      #KeyboardTable .keyboard { margin-bottom: 24px; }
    ` });
    await page.evaluate(() => document.fonts.ready);
    const sections = page.locator('#KeyboardTable > li');
    if (await sections.count() !== names.length) throw new Error('Expected six keyboard sections');
    for (const [index, name] of names.entries()) {
      await sections.nth(index).screenshot({ path: path.join(output, `${name}.png`) });
      console.log(`Exported ${name}.png`);
    }
  } finally {
    await browser.close();
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
