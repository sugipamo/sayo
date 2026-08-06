export function renderTermSource(term) {
  const root = element("span", "term-source term-fragment");
  decorate(root, term);
  appendTerm(root, term);
  return root;
}

export function termKind(term) {
  return term?.kind === "state" ? "state" : "plain";
}

function appendTerm(parent, term) {
  if (!term) {
    parent.textContent = "∅";
    return;
  }
  if (term.kind === "state") {
    parent.append(element("span", "term-state-head", term.head));
    parent.append(boundary("{", "state-brace brace-open"));
    appendItems(parent, term.payload ?? [], ",\u00a0");
    parent.append(boundary("}", "state-brace brace-close"));
    return;
  }
  if (term.kind === "group") {
    parent.append(boundary("(", "group-boundary paren-open"));
    appendItems(parent, term.items ?? [], "\u00a0");
    parent.append(boundary(")", "group-boundary paren-close"));
    return;
  }
  parent.textContent = term.source ?? term.name ?? "";
}

function appendItems(parent, items, separator) {
  items.forEach((item, index) => {
    if (index) parent.append(boundary(separator, "payload-separator"));
    const fragment = element("span", "term-fragment");
    decorate(fragment, item.term, item.view_id);
    appendTerm(fragment, item.term);
    parent.append(fragment);
  });
}

function decorate(element, term, viewId = null) {
  element.dataset.fragment = term?.source ?? "∅";
  if (term?.term_id != null) element.dataset.origin = String(term.term_id);
  if (viewId) element.dataset.viewId = viewId;
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function boundary(text, className) {
  return element("span", className, text);
}
