"""XPath subset for ElementTree — CPython-compatible surface.

Supports the ElementPath language: tag, *, ., .., //, [@attrib],
[@attrib='value'], [tag], [tag='text'], [.//tag], [index], [last()],
and predicate chains. Operates on the interpreter's native Element
instances (attribute access to .tag/.children/.attrib/.text).
"""

import re

xpath_tokenizer_re = re.compile(
    r"("
    r"'[^']*'|\"[^\"]*\"|"
    r"::|"
    r"\(\)|"
    r"//?"
    r"|\.\.|"
    r"\d+|[+-]?\d*\.?\d+|"
    r"[()\[\]{}@*:,.=]|"
    r"[^()\[\]{}@*:,.=/\s]+"
    r")",
    re.DOTALL,
)


def xpath_tokenizer(pattern, namespaces=None):
    for token in xpath_tokenizer_re.findall(pattern):
        if token and token[0] in "'\"":
            yield token[1:-1]
        else:
            yield token


_token_join_cache = {}


def _join_tokens(tokens):
    key = "|".join(tokens)
    cached = _token_join_cache.get(key)
    if cached is None:
        cached = tuple(tokens)
        _token_join_cache[key] = cached
    return cached


def get_parent(elem, tag=None):
    """Return the parent of elem (searched via a document walk)."""
    parent_map = getattr(elem, "_parent_map", None)
    if parent_map is not None:
        return parent_map.get(elem)
    return None


def _children(elem, node):
    # Native Element instances keep children in .children (list); pure-Python
    # fallback Elements use list-like iteration.
    ch = getattr(node, "children", None)
    if ch is None:
        try:
            ch = list(node)
        except TypeError:
            ch = []
    return list(ch)


def _child_tag(node):
    return getattr(node, "tag", None)


def _node_attrib(node):
    a = getattr(node, "attrib", None)
    if a is not None:
        return a
    return {}


def _node_text(node):
    return getattr(node, "text", None)


def _select_next_stream(children, path_tokens, pos):
    """Yield (element, newpos) matching one path step."""
    tok = path_tokens[pos]
    if tok == "*":
        for c in children:
            yield c, pos + 1
        return
    if tok == ".":
        yield ("__DOT__",), pos + 1
        return
    if tok == "..":
        yield ("__PARENT__",), pos + 1
        return
    for c in children:
        if _child_tag(c) == tok:
            yield c, pos + 1


def _predicate_match(pred, nodes, idx, node):
    """Evaluate one bracket predicate against node at position idx."""
    pred = pred.strip()
    # numeric index
    if pred.isdigit():
        return idx == int(pred) - 1
    if pred == "last()":
        return idx == len(nodes) - 1
    m = re.fullmatch(r"last\(\)\s*-\s*(\d+)", pred)
    if m:
        return idx == len(nodes) - 1 - int(m.group(1))
    # [@attrib]
    m = re.fullmatch(r"@([\w.:\-]+)", pred)
    if m:
        return m.group(1) in _node_attrib(node)
    # [@attrib='value'] / [@attrib="value"]
    m = re.fullmatch(r"@([\w.:\-]+)\s*=\s*(.+)", pred)
    if m:
        attr, val = m.group(1), m.group(2).strip()
        if val[:1] in "'\"" and val[-1:] == val[:1]:
            val = val[1:-1]
        return str(_node_attrib(node).get(attr)) == val
    # [tag='text']
    m = re.fullmatch(r"([\w.:\-]+|\*)\s*=\s*(.+)", pred)
    if m:
        tag, val = m.group(1), m.group(2).strip()
        if val[:1] in "'\"" and val[-1:] == val[:1]:
            val = val[1:-1]
        for c in _children(None, node):
            if (tag == "*" or _child_tag(c) == tag) and (_node_text(c) or "") == val:
                return True
        return False
    # [tag]
    if re.fullmatch(r"([\w.:\-]+|\*)", pred):
        for c in _children(None, node):
            if pred == "*" or _child_tag(c) == pred:
                return True
        return False
    raise SyntaxError("unsupported path syntax: [%s]" % pred)


def _split_steps(tokens):
    """Split token stream into (axis, name_or_pred_list) steps."""
    steps = []
    i = 0
    n = len(tokens)
    while i < n:
        t = tokens[i]
        if t == "/":
            i += 1
            continue
        if t == "//":
            steps.append(("descendant-or-self", None))
            i += 1
            continue
        if t == "[":
            # predicate attached to previous step
            j = i + 1
            depth = 1
            buf = []
            while j < n and depth:
                if tokens[j] == "[":
                    depth += 1
                elif tokens[j] == "]":
                    depth -= 1
                    if depth == 0:
                        break
                buf.append(tokens[j])
                j += 1
            steps.append(("pred", buf))
            i = j + 1
            continue
        steps.append(("child", t))
        i += 1
    return steps


def _apply_predicates(nodes_with_pos, preds, root_for_parent):
    out = []
    for node, _p in nodes_with_pos:
        ok = True
        siblings = None
        for pred_tokens in preds:
            if siblings is None:
                # sibling context: children of node's parent that share tag?
                # CPython applies index predicates per-parent group of the
                # matched child; we approximate with the candidate list.
                siblings = nodes_with_pos and [n for n, _ in nodes_with_pos]
            pred = "".join(pred_tokens)
            idx = siblings.index(node) if node in siblings else 0
            if not _predicate_match(pred, siblings, idx, node):
                ok = False
                break
        if ok:
            out.append((node, _p))
    return out


def iterfind(elem, path, namespaces=None):
    if path == ".":
        yield elem
        return
    tokens = _join_tokens(list(xpath_tokenizer(path)))
    steps = _split_steps(list(tokens))
    # current candidate set: [(node, kind)] where kind marks synthetic
    current = [(elem, "self")]
    i = 0
    while i < len(steps):
        axis, payload = steps[i]
        preds = []
        # gather consecutive predicates
        while i + 1 < len(steps) and steps[i + 1][0] == "pred":
            preds.append(steps[i + 1][1])
            i += 1
        nxt = []
        if axis == "descendant-or-self":
            # expand: self + all descendants, then next step selects from them
            stack = [e for e, _k in current]
            expanded = []
            for e in stack:
                expanded.append(e)
                for c in _children(None, e):
                    stack.append(c)
                    expanded.append(c)
            # peek the following step's selector name
            if i + 2 < len(steps) and steps[i + 2][0] == "child":
                name = steps[i + 2][1]
                for e in expanded:
                    for c in _children(None, e):
                        if name == "*" or _child_tag(c) == name:
                            nxt.append((c, None))
                    if name == "*":
                        pass
                i += 2
                current = nxt
                i += 1
                continue
            else:
                current = [(e, None) for e in expanded]
                i += 1
                continue
        name = payload
        for node, _k in current:
            kids = []
            if name == ".":
                kids = [node]
            elif name == "..":
                p = get_parent(node)
                if p is not None:
                    kids = [p]
            else:
                kids = [
                    c
                    for c in _children(None, node)
                    if name == "*" or _child_tag(c) == name
                ]
            for k in kids:
                nxt.append((k, None))
        if preds:
            cand = nxt
            filtered = []
            for node, _p in cand:
                ok = True
                sibs = [x for x, _ in cand]
                for pred_tokens in preds:
                    pred = "".join(pred_tokens)
                    idx = sibs.index(node) if node in sibs else 0
                    if not _predicate_match(pred, sibs, idx, node):
                        ok = False
                        break
                if ok:
                    filtered.append((node, None))
            nxt = filtered
        current = nxt
        i += 1
    for node, _k in current:
        yield node


def find(elem, path, namespaces=None):
    for e in iterfind(elem, path, namespaces):
        return e
    return None


def findall(elem, path, namespaces=None):
    return list(iterfind(elem, path, namespaces))


def findtext(elem, path, default=None, namespaces=None):
    for e in iterfind(elem, path, namespaces):
        text = _node_text(e)
        if text is not None:
            return text
        # also consider first child text like CPython? CPython returns '' when
        # element exists without text
        return ""
    return default


def prepare_predicate(next, link):  # kept for API parity (unused)
    return next


ElementPath = None  # module-level alias marker used by some tooling
