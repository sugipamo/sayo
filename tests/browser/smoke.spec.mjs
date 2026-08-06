import { expect, test } from "@playwright/test";

test("Sayo evaluates representative programs through Wasm and a Web Worker", async ({
  page,
}) => {
  const browserErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("pageerror", (error) => browserErrors.push(error.message));

  await page.goto("/smoke.html");
  await expect(page.locator("#status")).toHaveAttribute("data-result", "passed", {
    timeout: 30_000,
  });

  const results = JSON.parse(await page.locator("#results").textContent());
  expect(results).toEqual([
    expect.objectContaining({ name: "native relation", normal_form: "8" }),
    expect.objectContaining({ name: "empty relation directory", normal_form: "Boxed{5}" }),
    expect.objectContaining({ name: "embedded standard world", normal_form: "6" }),
  ]);
  expect(results.every((result) => result.visualization_frames > 0)).toBe(true);
  expect(browserErrors).toEqual([]);
});

test("Playground runs, replays, stops, edits a world, and shares it", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const browserErrors = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  await page.goto("/?autoplay=0");
  await page.locator("#sample-select").selectOption("motion");

  await expect(page.locator("#source-editor")).toHaveValue("for (add 9) 1 2 3");
  await expect(page.locator("#world-button")).toBeHidden();
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await expect(page.locator("#term-stage")).toContainText("for");
  await expect(page.locator("#term-stage")).toContainText("(add 9)");
  await expect(page.locator("#term-stage")).toContainText("1");
  await expect(page.locator("#timeline")).toHaveValue("0");
  const initialStageHeight = await page.locator("#stage").evaluate((stage) => stage.getBoundingClientRect().height);
  const event = (kind) => page.locator("#event-list li").filter({
    has: page.locator(".event-kind", { hasText: new RegExp(`^${kind}$`) }),
  });

  await event("control_started").first().evaluate((item) => item.click());
  await expect(page.locator('.nested-context[data-source^="EachApply"]'))
    .toContainText("EachApply{Plus{9}, 1}");
  await expect(page.locator(".nested-context")).toHaveCount(3);
  await expect(page.locator(".nested-context").nth(1)).toHaveText("2");
  await expect(page.locator(".nested-context").nth(2)).toHaveText("3");
  expect(await page.locator("#stage").evaluate((stage) => stage.getBoundingClientRect().height))
    .toBe(initialStageHeight);
  expect(await page.locator("#stage").evaluate((stage) => getComputedStyle(stage).overflowY))
    .toBe("auto");
  await page.setViewportSize({ width: 778, height: 720 });
  const contextGaps = await page.locator(".nested-context").evaluateAll((cards) => cards.slice(1).map((card, index) => {
    const previous = cards[index].getBoundingClientRect();
    return card.getBoundingClientRect().left - previous.right;
  }));
  expect(contextGaps.every((gap) => gap < 50)).toBe(true);
  await page.setViewportSize({ width: 1280, height: 720 });
  const reductionRows = page.locator(".nested-child-row");
  await expect(reductionRows.nth(0).locator(".nested-child:not(.leaving)"))
    .toHaveText("EachEmit{%0, (%0 %1)}");
  await page.locator("#step-forward").click();
  await expect(reductionRows).toHaveCount(2);
  await expect(reductionRows.nth(0).locator(".nested-child:not(.leaving)"))
    .toHaveText("EachEmit{%0, (%0 %1)}");
  await expect(reductionRows.nth(1).locator(".nested-child:not(.leaving)")).toHaveCount(2);
  await expect(reductionRows.nth(1).locator(".nested-child:not(.leaving)").nth(0)).toHaveText("%0");
  await expect(reductionRows.nth(1).locator(".nested-child:not(.leaving)").nth(1)).toHaveText("(%0 %1)");

  await event("control_slot_started").first().evaluate((item) => item.click());
  await expect(reductionRows).toHaveCount(3);
  await expect(reductionRows.nth(0).locator(".nested-child:not(.leaving)"))
    .toHaveText("EachEmit{%0, (%0 %1)}");
  await expect(reductionRows.nth(1).locator(".nested-child:not(.leaving)")).toHaveText(["Plus{9}", "(%0 %1)"]);
  await expect(reductionRows.nth(2).locator(".nested-child:not(.leaving)")).toHaveText(["%0", "%1"]);
  await expect(reductionRows.nth(0).locator(".nested-child:not(.leaving)")).toHaveCount(1);

  await event("control_slot_resolved").first().evaluate((item) => item.click());
  await expect(reductionRows.nth(0).locator(".nested-child:not(.leaving)"))
    .toHaveText("EachEmit{%0, (%0 %1)}");
  await expect(reductionRows.nth(1).locator(".nested-child:not(.leaving)")).toHaveText(["Plus{9}", "10"]);
  await expect(reductionRows.nth(2).locator(".nested-child:not(.leaving)")).toHaveText("10");

  await event("control_completed").first().evaluate((item) => item.click());
  await expect(reductionRows).toHaveCount(2);
  await expect(reductionRows.nth(1).locator(".nested-child:not(.leaving)")).toHaveText(["Plus{9}", "10"]);

  await event("control_assembled").first().evaluate((item) => item.click());
  await expect(reductionRows).toHaveCount(1);
  await expect(reductionRows.nth(0).locator(".nested-child:not(.leaving)"))
    .toHaveText("EachEmit{Plus{9}, 10}");

  await event("control_returned").first().evaluate((item) => item.click());
  await expect(page.locator('.term-card:not(.leaving)[data-source^="EachEmit"]')).toHaveCount(1);
  await page.locator("#jump-end").click();
  await expect(page.locator("#term-stage")).toContainText("10");
  await expect(page.locator("#term-stage")).toContainText("11");
  await expect(page.locator("#term-stage")).toContainText("12");
  await expect(page.locator("#event-list li")).not.toHaveCount(0);
  await expect(page.locator("#term-stage")).not.toContainText("…");
  await expect(page.locator('.term-card[data-kind="plain"]')).toHaveCount(3);
  await expect(page.locator('.term-card[data-kind="state"]')).toHaveCount(1);
  await expect(page.locator("#term-stage .term-cell")).toHaveCount(0);
  await expect(page.locator("#term-stage .term-space")).toHaveCount(0);
  expect(await page.locator(".term-card").first().evaluate((card) => getComputedStyle(card).transitionDuration)).not.toBe("0s");

  const lastStep = await page.locator("#timeline").inputValue();
  await page.locator("#jump-start").click();
  await expect(page.locator("#timeline")).toHaveValue("0");
  await page.locator("#jump-end").click();
  await expect(page.locator("#timeline")).toHaveValue(lastStep);
  await page.locator("#step-back").click();
  expect(Number(await page.locator("#timeline").inputValue())).toBe(Number(lastStep) - 1);
  await expect(page.locator(".morph-ghost")).toHaveCount(0);
  await expect(page.locator(".term-card.leaving")).toHaveCount(0);
  await page.locator("#play-button").click();
  await expect(page.locator("#play-button")).toHaveText("Ⅱ");
  await page.locator("#play-button").click();

  await page.locator("#world-button").evaluate((button) => button.click());
  await expect(page.locator("#world-dialog")).toBeVisible();
  await page.locator("#add-file").click();
  const cards = page.locator(".file-card");
  await cards.last().locator(".file-path").fill("note.sayo");
  await cards.last().locator(".file-source").fill("5");
  await page.locator("#save-world").click();
  await expect(page.locator("#world-summary")).toContainText("2 files");

  await page.locator("#share-button").click();
  await expect(page).toHaveURL(/#p=/);
  expect((await page.evaluate(() => location.href)).length).toBeLessThan(8000);
  const sharedUrl = await page.evaluate(() => location.href);
  await page.goto(sharedUrl);
  await expect(page.locator("#world-summary")).toContainText("2 files");
  await expect(page.locator("#source-editor")).toHaveValue("for (add 9) 1 2 3");
  expect(browserErrors).toEqual([]);
});

test("Playground opens with the composite showcase and plays it once", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#source-editor"))
    .toHaveValue("for (add (fold add 0 1 2 3)) 4 5 6 7");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await expect(page.locator("#play-button")).toHaveText("Ⅱ", { timeout: 5_000 });
  await expect(page.locator("#speed-select")).toHaveValue("150");
  await expect(page.locator("#speed-select option:checked")).toHaveText("2×");
  await expect.poll(async () => Number(await page.locator("#timeline").inputValue()))
    .toBeGreaterThan(0);
  await page.locator("#play-button").click();

  await expect(page.locator(".example-card")).toHaveCount(5);
  await page.locator('.example-card[data-sample="arithmetic"]').click();
  await expect(page.locator("#source-editor")).toHaveValue("add 5 3");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await expect(page.locator("#play-button")).toHaveText("Ⅱ", { timeout: 5_000 });
  await page.locator("#play-button").click();
});

test("Nested Groups retain every outer expression as a separate row", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.locator("#event-list li").nth(2).evaluate((item) => item.click());

  const rows = page.locator("#term-stage > .nested-context-row, #term-stage > .nested-child-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText("for");
  await expect(rows.nth(0)).toContainText("4");
  await expect(rows.nth(1)).toContainText("add");
  await expect(rows.nth(1)).toContainText("fold");
  await expect(rows.nth(2)).toContainText("fold");
  await expect(rows.nth(2)).toContainText("3");
});

test("Reduction scrolling follows the deepest active row and offscreen rewrites", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  await page.goto("/?autoplay=0");
  await expect(page.locator("#status-pill")).toHaveText("Normal");

  const nestedControl = page.locator("#event-list li").filter({
    has: page.locator(".event-kind", { hasText: /^control_slot_started$/ }),
  }).first();
  await nestedControl.evaluate((item) => item.click());
  await expect.poll(() => page.locator("#stage").evaluate((stage) => stage.scrollTop))
    .toBeGreaterThan(0);
  const verticalPosition = await page.locator("#stage").evaluate((stage) => {
    const rows = stage.querySelectorAll("#term-stage > .nested-child-row");
    const row = rows[rows.length - 1];
    const viewport = stage.getBoundingClientRect();
    const target = row.getBoundingClientRect();
    return { targetBottom: target.bottom, viewportBottom: viewport.bottom };
  });
  expect(verticalPosition.targetBottom).toBeLessThanOrEqual(verticalPosition.viewportBottom);

  const longPrefix = Array.from({ length: 20 }, (_, index) => index + 1).join(" ");
  await page.locator("#source-editor").fill(`${longPrefix} add 5 3`);
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  const applied = page.locator("#event-list li").filter({
    has: page.locator(".event-kind", { hasText: /^relation_applied$/ }),
  }).last();
  await applied.evaluate((item) => item.click());
  await expect.poll(() => page.locator("#stage").evaluate((stage) => stage.scrollLeft))
    .toBeGreaterThan(0);
});

test("Playground respects reduced motion when considering initial autoplay", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.waitForTimeout(900);
  await expect(page.locator("#timeline")).toHaveValue("0");
  await expect(page.locator("#play-button")).toHaveText("▶");
});

test("Playground renders traps without losing their trace", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await page.locator("#source-editor").fill("div 1 0");
  await page.locator("#play-button").click();
  await expect(page.locator("#status-pill")).toHaveText("trap", { ignoreCase: true });
  await expect(page.locator("#toast")).not.toContainText("division by zero");
  await page.locator("#jump-end").click();
  await expect(page.locator("#toast")).toContainText("division by zero");
  await expect(page.locator(".term-card.trapped")).toHaveCount(2);
  expect(await page.locator(".term-card.trapped").first().evaluate((term) => getComputedStyle(term).animationName)).toBe("trap-shake");
  await expect(page.locator("#event-list li")).not.toHaveCount(0);
});

test("Playground can stop a Worker and import a project document", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.locator("#source-editor").fill("fold add 0 1 2 3 4 5 6 7 8 9");
  await page.locator("#play-button").evaluate((button) => {
    button.click();
    button.click();
  });
  await expect(page.locator("#status-pill")).toHaveText("Stopped");
  await expect(page.locator("#play-button")).toHaveAttribute("aria-label", "Run Sayo program");

  await page.locator("#world-button").evaluate((button) => button.click());
  await page.locator("#file-input").setInputFiles({
    name: "shared-world.json",
    mimeType: "application/json",
    buffer: Buffer.from(JSON.stringify({
      format: "sayo-playground",
      version: 0,
      entry: "program/main.sayo",
      files: { "program/main.sayo": "add 20 22", "note.sayo": "5", "Box/space.sayo": 'equal-to "box" %0' },
      directories: ["Box/Int/Boxed", "Boxed"],
    })),
  });
  await expect(page.locator("#source-editor")).toHaveValue("add 20 22");
  await expect(page.locator("#world-summary")).toContainText("3 files · 2 empty dirs");
  await page.locator("#play-button").click();
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.locator("#jump-end").click();
  await expect(page.locator("#term-stage .term-source", { hasText: /^42$/ })).toHaveCount(1);

  await page.locator("#world-button").evaluate((button) => button.click());
  const downloadPromise = page.waitForEvent("download");
  await page.locator("#export-button").click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe("sayo-world.json");
});

test("Playground refuses an oversized share URL and stays usable on mobile", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/?autoplay=0");
  const noisySource = await page.evaluate(() => {
    const bytes = crypto.getRandomValues(new Uint8Array(50000));
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary);
  });
  await page.locator("#source-editor").fill(noisySource);
  await page.locator("#share-button").evaluate((button) => button.click());
  await expect(page.locator("#toast")).toContainText("Too large");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
});

test("Visualizer follows source edits without replaying a no-op scan", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await expect(page.locator("#speed-select")).toHaveValue("220");
  await expect(page.locator("#speed-select option:checked")).toHaveText("1×");

  const longSource = Array.from({ length: 40 }, (_, index) => index + 1).join(" ");
  await page.locator("#source-editor").fill(longSource);
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await expect(page.locator("#term-stage .term-source", { hasText: /^40$/ })).toHaveCount(1);
  expect(await page.locator("#stage").evaluate((stage) => getComputedStyle(stage).overflowX)).toBe("auto");
  expect(await page.locator("#stage").evaluate((stage) => stage.scrollWidth > stage.clientWidth)).toBe(true);

  await page.locator("#jump-end").click();
  await expect(page.locator("#event-list li")).toHaveCount(1);
  expect(await page.locator("#stage").evaluate((stage) => stage.scrollLeft)).toBe(0);
});

test("Continuous playback settles each animation inside its frame budget", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.locator("#sample-select").selectOption("motion");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.locator("#event-list li").nth(20).evaluate((item) => item.click());
  await page.locator("#speed-select").selectOption("45");
  await page.waitForTimeout(400);

  const observedDuration = page.evaluate(() => new Promise((resolve) => {
    let maximum = 0;
    const sample = setInterval(() => {
      const stage = document.querySelector("#term-stage");
      const animations = [
        ...stage.getAnimations({ subtree: true }),
        ...[...document.querySelectorAll(".morph-ghost")]
          .flatMap((ghost) => ghost.getAnimations()),
      ];
      for (const animation of animations) {
        const duration = animation.effect?.getTiming().duration;
        if (typeof duration === "number") maximum = Math.max(maximum, duration);
      }
    }, 4);
    setTimeout(() => {
      clearInterval(sample);
      resolve(maximum);
    }, 300);
  }));

  await page.locator("#play-button").click();
  expect(await observedDuration).toBeLessThanOrEqual(24);
  if (await page.locator("#play-button").textContent() === "Ⅱ") {
    await page.locator("#play-button").click();
  }
  await page.waitForTimeout(40);
  await expect(page.locator(".morph-ghost")).toHaveCount(0);
});

test("Switching examples discards playback and prepares the new initial frame", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await page.locator("#play-button").click();
  await expect(page.locator("#play-button")).toHaveText("Ⅱ");

  await page.locator("#sample-select").selectOption("arithmetic");
  await expect(page.locator("#source-editor")).toHaveValue("add 5 3");
  await expect(page.locator("#status-pill")).toHaveText("Normal");
  await expect(page.locator("#timeline")).toHaveValue("0");
  await expect(page.locator("#play-button")).toHaveText("▶");
  await expect(page.locator("#term-stage")).toHaveText("add53");
  await expect(page.locator("#term-stage")).not.toContainText("Each");
  await expect(page.locator(".term-card.leaving")).toHaveCount(0);
  await expect(page.locator(".morph-ghost")).toHaveCount(0);
});

test("A relation returning a plain value crossfades its operands and result", async ({ page }) => {
  await page.goto("/?autoplay=0");
  await page.locator("#sample-select").selectOption("arithmetic");
  await expect(page.locator("#status-pill")).toHaveText("Normal");

  const applied = page.locator("#event-list li").filter({
    has: page.locator(".event-kind", { hasText: /^relation_applied$/ }),
  });
  const finalIndex = await applied.last().evaluate((item) => [...item.parentElement.children].indexOf(item));
  await page.locator("#event-list li").nth(finalIndex - 1).evaluate((item) => item.click());
  await expect(page.locator("#term-stage")).toContainText("Plus{5}");
  await expect(page.locator("#term-stage")).toContainText("3");

  await page.locator("#step-forward").click();
  const leaving = page.locator('.term-card.leaving[data-source="Plus{5}"], .term-card.leaving[data-source="3"]');
  await expect(leaving).toHaveCount(2);
  await expect.poll(() => leaving.evaluateAll((cards) => cards.reduce(
    (count, card) => count + card.getAnimations().length,
    0,
  ))).toBeGreaterThan(0);
  await expect(page.locator('.term-card:not(.leaving)[data-source="8"]')).toHaveCount(1);
  expect(await leaving.evaluateAll((cards) => cards.every((card) => card.getAnimations().every((animation) => {
    const frames = animation.effect.getKeyframes();
    return frames.some((frame) => frame.opacity === "0")
      && frames.every((frame) => !frame.transform || frame.transform === "none");
  })))).toBe(true);
});
