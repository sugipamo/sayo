const MOTION = {
  arrival: 400,
  carried: 420,
  fusion: 430,
};

const MOTION_CAUSES = new Set([
  "evaluation_started",
  "control_started",
  "control_slot_started",
  "relation_applied",
  "state_changed",
  "end_changed",
  "control_payload_exposed",
  "control_slot_resolved",
  "control_completed",
  "control_assembled",
  "control_returned",
]);

export function snapshotFragments(stage) {
  return [...stage.querySelectorAll(".term-card:not(.leaving):not(.nested-context) .term-fragment")]
    .filter((fragment) => fragment.dataset.viewId)
    .map((fragment) => ({
      viewId: fragment.dataset.viewId,
      rect: fragment.getBoundingClientRect(),
      color: getComputedStyle(fragment).color,
      text: fragment.textContent,
    }));
}

export function resetMotion(stage) {
  stage.getAnimations({ subtree: true }).forEach((animation) => animation.cancel());
  stage.querySelectorAll(".morph-target").forEach((target) => target.classList.remove("morph-target"));
  stage.querySelectorAll(".morph-ghost").forEach((ghost) => {
    ghost.getAnimations().forEach((animation) => animation.cancel());
    ghost.remove();
  });
}

export function animateRewrite(frame, stage, oldFragments, { maxDuration = Infinity } = {}) {
  if (!MOTION_CAUSES.has(frame.cause.kind)) return;
  const motion = transitionContext(frame, stage);
  animateArrival(motion.destinationCards, motionDuration(MOTION.arrival, maxDuration));

  if (shouldFuse(frame, motion)) {
    animateFusion(motion, motionDuration(MOTION.fusion, maxDuration));
  } else {
    motion.removedCards.forEach((card) => {
      card.style.opacity = "0";
    });
  }
  animatePreservedFragments(
    frame.changes,
    oldFragments,
    stage,
    motionDuration(MOTION.carried, maxDuration),
  );
}

function shouldFuse(frame, motion) {
  if (frame.cause.kind !== "relation_applied" || motion.destinationCards.length !== 1) return false;
  const head = motion.destinationCards[0].querySelector(".term-state-head")?.textContent ?? "";
  return head.length > 0 && !head.startsWith("Each");
}

function transitionContext(frame, stage) {
  const changed = frame.changes.filter((change) => change.kind !== "preserve");
  const destinationCards = unique(changed.flatMap((change) => change.to)
    .map((viewId) => stage.querySelector(`.term-card[data-key="${CSS.escape(viewId)}"]:not(.leaving)`))
    .filter(Boolean));
  const removedCards = [...stage.querySelectorAll(".term-card.leaving")];
  return {
    destinationCards,
    removedCards,
  };
}

function animateArrival(cards, duration) {
  for (const card of cards) {
    card.animate(
      [{ opacity: .15, filter: "blur(6px)" }, { opacity: 1, filter: "blur(0)" }],
      { duration, easing: "cubic-bezier(.16,.82,.28,1)" },
    );
  }
}

function animateFusion(motion, duration) {
  const head = motion.destinationCards[0]?.querySelector(".term-state-head");
  if (!head) return;
  const target = head.getBoundingClientRect();
  const targetX = centerX(target);
  const targetY = centerY(target);
  for (const card of motion.removedCards) {
    prepareLeavingCard(card);
    const rect = card.getBoundingClientRect();
    const translation = `translate(${targetX - centerX(rect)}px, ${targetY - centerY(rect)}px)`;
    card.animate([
      { transform: "translate(0, 0)", opacity: 1, filter: "blur(0)" },
      { transform: translation, opacity: 1, filter: "blur(0)", offset: .78 },
      { transform: translation, opacity: 0, filter: "blur(4px)" },
    ], { duration, easing: "cubic-bezier(.2,.75,.25,1)", fill: "forwards" });
  }
}

function animatePreservedFragments(changes, oldFragments, stage, duration) {
  const positions = new Map(oldFragments.map((fragment) => [fragment.viewId, fragment]));
  for (const change of changes) {
    if (change.kind !== "preserve") continue;
    change.from.forEach((from, index) => {
      const origin = positions.get(from);
      const to = change.to[index];
      if (!origin || !to) return;
      const target = stage.querySelector(`.term-fragment[data-view-id="${CSS.escape(to)}"]`);
      if (!target) return;
      const targetRect = settledTargetRect(target);
      if (samePosition(origin.rect, targetRect)) return;
      animateCarriedFragment(origin, target, targetRect, stage, duration);
    });
  }
}

function animateCarriedFragment(origin, target, targetRect, stage, duration) {
  const ghost = document.createElement("span");
  ghost.className = "morph-ghost";
  ghost.textContent = origin.text;
  ghost.dataset.viewId = origin.viewId;
  const originInStage = relativePosition(origin.rect, stage.getBoundingClientRect());
  Object.assign(ghost.style, {
    left: `${originInStage.left}px`,
    top: `${originInStage.top}px`,
    width: `${origin.rect.width}px`, height: `${origin.rect.height}px`, color: origin.color,
  });
  // Keep carried fragments in the same coordinate system as the terms. The
  // Reduction viewport may auto-scroll during playback; a body-fixed ghost
  // would stay behind and briefly appear over an unrelated row.
  stage.append(ghost);
  target.classList.add("morph-target");
  const animation = ghost.animate([
    { transform: "translate(0, 0)", opacity: 1 },
    { transform: `translate(${targetRect.left - origin.rect.left}px, ${targetRect.top - origin.rect.top}px)`, opacity: 1 },
  ], { duration, easing: "cubic-bezier(.2,.75,.25,1)", fill: "forwards" });
  const cleanup = () => {
    ghost.remove();
    target.classList.remove("morph-target");
  };
  animation.finished.then(cleanup, cleanup);
}

function prepareLeavingCard(card) {
  card.style.opacity = "1";
  card.style.filter = "none";
}

// FLIP keeps a moved card visually at its previous row while its transform eases
// toward the new layout. Fragment handoffs must aim at that settled layout, not
// at the card's temporary transformed position between the two rows.
function settledTargetRect(target) {
  const rect = target.getBoundingClientRect();
  const card = target.closest(".term-card");
  if (!card) return rect;
  const transform = getComputedStyle(card).transform;
  if (!transform || transform === "none") return rect;
  const matrix = new DOMMatrixReadOnly(transform);
  return {
    left: rect.left - matrix.m41,
    right: rect.right - matrix.m41,
    top: rect.top - matrix.m42,
    bottom: rect.bottom - matrix.m42,
    width: rect.width,
    height: rect.height,
  };
}

function unique(items) { return [...new Set(items)]; }
function relativePosition(rect, ancestorRect) {
  return { left: rect.left - ancestorRect.left, top: rect.top - ancestorRect.top };
}
function motionDuration(naturalDuration, maxDuration) { return Math.min(naturalDuration, maxDuration); }
function samePosition(left, right) { return Math.abs(left.left - right.left) < 2 && Math.abs(left.top - right.top) < 2; }
function centerX(rect) { return rect.left + rect.width / 2; }
function centerY(rect) { return rect.top + rect.height / 2; }
