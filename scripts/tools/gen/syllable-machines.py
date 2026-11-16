#!/usr/bin/env python3

import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from categories import INDIC_CLASSES
from rows import wrapped

HERE = os.path.dirname(os.path.abspath(__file__))
GRAMMARS = os.path.join(HERE, "..", "..", "data", "grammars")
OUT = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "syllable_tables.rs")

def parse_grammar(path):
    categories, definitions, syllables = {}, {}, []
    section = None

    with open(path, encoding="utf-8") as f:
        for lineno, raw in enumerate(f, 1):
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue

            if line.endswith(":") and " " not in line:
                section = line[:-1]
                if section not in ("categories", "definitions", "syllables"):
                    sys.exit("{}:{}: unknown section {!r}".format(path, lineno, section))
                continue

            if section is None:
                sys.exit("{}:{}: content before any section".format(path, lineno))

            if "=" not in line:
                sys.exit("{}:{}: expected 'name = ...'".format(path, lineno))
            name, body = (p.strip() for p in line.split("=", 1))

            if section == "categories":
                categories[name] = int(body)
            elif section == "definitions":
                definitions[name] = body
            else:
                syllables.append((name, body))

    return categories, definitions, syllables

class Node:
    pass

class Sym(Node):
    def __init__(self, value):
        self.value = value

class Cat(Node):  # concatenation
    def __init__(self, parts):
        self.parts = parts

class Alt(Node):
    def __init__(self, parts):
        self.parts = parts

class Rep(Node):
    def __init__(self, node, min_count, max_count):
        self.node = node
        self.min = min_count
        self.max = max_count  # None means unbounded

TOKEN = re.compile(r"\s*(\(|\)|\||\?|\*|\+|\.|[A-Za-z_][A-Za-z_0-9]*)")

def tokenize(text):
    out, at = [], 0
    while at < len(text):
        m = TOKEN.match(text, at)
        if not m:
            sys.exit("cannot tokenize {!r} at {}".format(text, at))
        out.append(m.group(1))
        at = m.end()
    return out

class Parser:

    def __init__(self, tokens, categories, definitions, width, expanding=()):
        self.tokens = tokens
        self.at = 0
        self.categories = categories
        self.definitions = definitions
        self.width = width
        self.expanding = expanding

    def peek(self):
        return self.tokens[self.at] if self.at < len(self.tokens) else None

    def take(self):
        tok = self.peek()
        self.at += 1
        return tok

    def parse(self):
        node = self.alternation()
        if self.peek() is not None:
            sys.exit("trailing {!r} in expression".format(self.peek()))
        return node

    def alternation(self):
        parts = [self.concatenation()]
        while self.peek() == "|":
            self.take()
            parts.append(self.concatenation())
        return parts[0] if len(parts) == 1 else Alt(parts)

    def concatenation(self):
        parts = []
        while True:
            tok = self.peek()
            if tok is None or tok in (")", "|"):
                break
            if tok == ".":
                self.take()
                continue
            parts.append(self.repeat())
        if not parts:
            return Cat([])
        return parts[0] if len(parts) == 1 else Cat(parts)

    def repeat(self):
        node = self.atom()
        while self.peek() in ("?", "*", "+"):
            op = self.take()
            if op == "?":
                node = Rep(node, 0, 1)
            elif op == "*":
                node = Rep(node, 0, None)
            else:
                node = Rep(node, 1, None)
        return node

    def atom(self):
        tok = self.take()
        if tok == "any":
            return Alt([Sym(c) for c in range(self.width)])
        if tok == "(":
            node = self.alternation()
            if self.take() != ")":
                sys.exit("unbalanced parenthesis")
            return node
        if tok in self.categories:
            return Sym(self.categories[tok])
        if tok in self.definitions:
            if tok in self.expanding:
                sys.exit("definition {!r} is recursive, which a regular grammar cannot be".format(tok))
            return parse_expression(
                self.definitions[tok], self.categories, self.definitions, self.width, self.expanding + (tok,)
            )
        sys.exit("unknown symbol {!r}".format(tok))

def parse_expression(text, categories, definitions, width, expanding=()):
    return Parser(tokenize(text), categories, definitions, width, expanding).parse()

class Nfa:
    def __init__(self):
        self.transitions = []  # state -> list of (category or None, target)

    def new_state(self):
        self.transitions.append([])
        return len(self.transitions) - 1

    def link(self, frm, to, category=None):
        self.transitions[frm].append((category, to))

    def build(self, node, start):
        if isinstance(node, Sym):
            end = self.new_state()
            self.link(start, end, node.value)
            return end

        if isinstance(node, Cat):
            at = start
            for part in node.parts:
                at = self.build(part, at)
            return at

        if isinstance(node, Alt):
            end = self.new_state()
            for part in node.parts:
                branch = self.new_state()
                self.link(start, branch)
                self.link(self.build(part, branch), end)
            return end

        if isinstance(node, Rep):
            at = start
            for _ in range(node.min):
                at = self.build(node.node, at)

            if node.max is None:
                loop = self.new_state()
                self.link(at, loop)
                body_end = self.build(node.node, loop)
                self.link(body_end, loop)
                return loop

            end = self.new_state()
            self.link(at, end)
            for _ in range(node.max - node.min):
                at = self.build(node.node, at)
                self.link(at, end)
            return end

        sys.exit("unknown node {!r}".format(node))

    def epsilon_closure(self, states):
        stack, seen = list(states), set(states)
        while stack:
            state = stack.pop()
            for category, target in self.transitions[state]:
                if category is None and target not in seen:
                    seen.add(target)
                    stack.append(target)
        return frozenset(seen)

    def step(self, states, category):
        out = set()
        for state in states:
            for edge_category, target in self.transitions[state]:
                if edge_category == category:
                    out.add(target)
        return self.epsilon_closure(out) if out else frozenset()

def syllable_types(syllables):
    out = []
    for name, _ in syllables:
        if name not in out:
            out.append(name)
    return out

def compile_machine(categories, definitions, syllables, width):
    types = syllable_types(syllables)
    rule_type = [types.index(name) for name, _ in syllables]
    nfa = Nfa()
    start = nfa.new_state()
    accept_rule = {}

    for index, (_, body) in enumerate(syllables):
        branch = nfa.new_state()
        nfa.link(start, branch)
        end = nfa.build(parse_expression(body, categories, definitions, width), branch)
        accept_rule.setdefault(end, index)
        if accept_rule[end] > index:
            accept_rule[end] = index

    alphabet = range(width)

    initial = nfa.epsilon_closure({start})
    states = {initial: 0}
    order = [initial]
    transitions = []
    accepts = []

    at = 0
    while at < len(order):
        current = order[at]
        row = [DEAD] * width
        for category in alphabet:
            target = nfa.step(current, category)
            if not target:
                continue
            if target not in states:
                states[target] = len(order)
                order.append(target)
            row[category] = states[target]
        transitions.append(row)

        winning = [accept_rule[s] for s in current if s in accept_rule]
        accepts.append(rule_type[min(winning)] if winning else None)
        at += 1

    return transitions, accepts

DEAD = 0xFFFF

def minimize(transitions, accepts):
    width = len(transitions[0])
    count = len(transitions)

    group_of = {}
    for state in range(count):
        group_of[state] = accepts[state]
    groups = {}
    for state, key in group_of.items():
        groups.setdefault(key, []).append(state)
    partition = {state: index for index, key in enumerate(sorted(groups, key=lambda k: (k is not None, k)))
                 for state in groups[key]}

    while True:
        signatures = {}
        for state in range(count):
            row = transitions[state]
            signature = (partition[state],) + tuple(
                -1 if row[c] == DEAD else partition[row[c]] for c in range(width)
            )
            signatures.setdefault(signature, []).append(state)

        if len(signatures) == len(set(partition.values())):
            break

        partition = {}
        for index, signature in enumerate(sorted(signatures)):
            for state in signatures[signature]:
                partition[state] = index

    order = []
    seen = {}
    for state in range(count):
        group = partition[state]
        if group not in seen:
            seen[group] = len(order)
            order.append(state)

    new_transitions = []
    new_accepts = []
    for representative in order:
        row = transitions[representative]
        new_transitions.append([
            DEAD if row[c] == DEAD else seen[partition[row[c]]] for c in range(width)
        ])
        new_accepts.append(accepts[representative])

    return new_transitions, new_accepts

def run(transitions, accepts, categories):
    out, at = [], 0
    while at < len(categories):
        state, last, i = 0, None, at
        while i < len(categories):
            nxt = transitions[state][categories[i]]
            if nxt == DEAD:
                break
            state = nxt
            i += 1
            if accepts[state] is not None:
                last = (i, accepts[state])
        if last is None:
            at += 1
            continue
        out.append((at, last[0], last[1]))
        at = last[0]
    return out

def live(transitions, accepts):
    # The states some accepting state can be reached from; the rest end every syllable as a dead end does.
    into = [[] for _ in transitions]
    for state, row in enumerate(transitions):
        for target in row:
            if target != DEAD:
                into[target].append(state)
    alive = {state for state, label in enumerate(accepts) if label is not None}
    stack = list(alive)
    while stack:
        for state in into[stack.pop()]:
            if state not in alive:
                alive.add(state)
                stack.append(state)
    return alive

def difference(before, after, width):
    # The shortest category sequence the two scanners label differently, or None. Scanners that agree on every
    # sequence find the same syllables in any text, each taking the longest match.
    from collections import deque
    machines = [(t, a, live(t, a)) for t, a in (before, after)]
    def step(machine, state, category):
        transitions, _, alive = machine
        target = DEAD if state == DEAD else transitions[state][category]
        return target if target in alive else DEAD
    def label(machine, state):
        return None if state == DEAD else machine[1][state]
    seen = {(0, 0): None}
    queue = deque([(0, 0)])
    while queue:
        pair = queue.popleft()
        if label(machines[0], pair[0]) != label(machines[1], pair[1]):
            path, at = [], pair
            while seen[at] is not None:
                at, category = seen[at]
                path.append(category)
            return path[::-1], label(machines[0], pair[0]), label(machines[1], pair[1])
        if pair == (DEAD, DEAD):
            continue
        for category in range(width):
            nxt = (step(machines[0], pair[0], category), step(machines[1], pair[1], category))
            if nxt not in seen:
                seen[nxt] = (pair, category)
                queue.append(nxt)
    return None

def compare(old_path, new_path):
    # Whether two grammars for one script find the same syllables, as named kinds, in every text.
    use_categories, _, _ = parse_grammar(os.path.join(GRAMMARS, "use.grammar"))
    machines = []
    for path in (old_path, new_path):
        categories, definitions, syllables = parse_grammar(path)
        name = os.path.basename(path)[:-8]
        width = alphabet_width(name, categories, use_categories)
        transitions, accepts = compile_machine(categories, definitions, syllables, width)
        kinds = syllable_types(syllables)
        machines.append(((transitions, [None if a is None else kinds[a] for a in accepts]), categories, width))
    names = {number: cat for cat, number in machines[0][1].items()}
    names.update({number: cat for cat, number in machines[1][1].items()})
    found = difference(machines[0][0], machines[1][0], max(machines[0][2], machines[1][2]))
    if found is None:
        print("  same syllables in every text")
        return True
    path, old, new = found
    print("  differ on {}: {} before, {} after".format(" ".join(names.get(c, str(c)) for c in path), old, new))
    return False

def emit(f, name, kinds, syllables, transitions, accepts):
    upper = name.upper()
    width = len(transitions[0])
    types = syllable_types(syllables)
    kind = kinds.capitalize()

    if kinds != name:
        emit_machine(f, name, upper, kind, width, transitions, accepts)
        return

    f.write("// `{}` syllable types, in the grammar's own order.\n".format(name))
    f.write("#[derive(Clone, Copy, PartialEq, Eq, Debug)]\n")
    f.write("#[allow(clippy::enum_variant_names)]\n")
    f.write("pub(crate) enum {}Syllable {{\n".format(name.capitalize()))
    for rule_name in types:
        f.write("    {},\n".format(to_camel(rule_name)))
    f.write("}\n\n")

    f.write("impl {}Syllable {{\n".format(name.capitalize()))
    f.write("    fn from_index(i: u8) -> Option<Self> {\n        Some(match i {\n")
    for index, rule_name in enumerate(types):
        f.write("            {} => {}Syllable::{},\n".format(index, name.capitalize(), to_camel(rule_name)))
    f.write("            _ => return None,\n        })\n    }\n}\n\n")

    f.write("impl From<{}Syllable> for u8 {{\n".format(name.capitalize()))
    f.write("    fn from(s: {}Syllable) -> u8 {{\n        match s {{\n".format(name.capitalize()))
    for index, rule_name in enumerate(types):
        f.write("            {}Syllable::{} => {},\n".format(name.capitalize(), to_camel(rule_name), index))
    f.write("        }\n    }\n}\n\n")

    emit_machine(f, name, upper, kind, width, transitions, accepts)

def emit_machine(f, name, upper, kind, width, transitions, accepts):
    if len(transitions) >= 0xFF:
        sys.exit("{}: {} states, where a u8 state leaves room for 254 and DEAD".format(name, len(transitions)))
    f.write("// `[state][category]` -> next state, or `DEAD`.\n")
    f.write("pub(crate) static {}_TRANSITIONS: [[u8; {}]; {}] = [\n".format(upper, width, len(transitions)))
    for row in transitions:
        cells = ["DEAD" if v == DEAD else str(v) for v in row]
        one = "    [{}],".format(", ".join(cells))
        if len(one) <= 120:
            f.write(one + "\n")
        else:
            f.write("    [\n{}\n    ],\n".format("\n".join(wrapped(" " * 8, [c + "," for c in cells]))))
    f.write("];\n\n")

    f.write("// Which syllable type each state accepts, if any.\n")
    f.write("pub(crate) static {}_ACCEPTS: [u8; {}] = [\n".format(upper, len(accepts)))
    f.write("\n".join(wrapped("    ", ["NONE," if a is None else "{},".format(a) for a in accepts])))
    f.write("\n];\n\n")

    f.write("pub(crate) fn {}_accept(state: u8) -> Option<{}Syllable> {{\n".format(name, kind))
    f.write("    {}Syllable::from_index(*{}_ACCEPTS.get(state as usize)?)\n}}\n\n".format(kind, upper))

# Each machine reads the categories its table gives, so it spans all of them, and `any` takes in the ones it
# names no rule for. A grammar's numbers must be its table's.
TABLE_OF = {"indic": "indic", "khmer": "indic", "myanmar": "indic", "use": "use", "lana": "use"}
ALIASES = {"myanmar": {"IV": "V", "DB": "N", "GB": "Placeholder"}}

def alphabet_width(name, categories, use_categories):
    table = TABLE_OF.get(name)
    if table == "use":
        if categories != use_categories:
            sys.exit("{}: categories differ from use.grammar's, which the USE table is built from".format(name))
        return max(use_categories.values()) + 1
    if table != "indic":
        sys.exit("{}: no category table is known for this grammar".format(name))
    for cat, number in categories.items():
        listed = INDIC_CLASSES[number] if number < len(INDIC_CLASSES) else None
        if listed != ALIASES.get(name, {}).get(cat, cat):
            sys.exit("{}: {} = {}, where the Indic table has {} there".format(name, cat, number, listed))
    return len(INDIC_CLASSES)

def to_camel(name):
    return "".join(part.capitalize() for part in name.split("_"))

def main():
    if not os.path.isdir(GRAMMARS):
        sys.exit("missing {}".format(GRAMMARS))

    names = sorted(n[:-8] for n in os.listdir(GRAMMARS) if n.endswith(".grammar"))
    if not names:
        sys.exit("no grammars in {}".format(GRAMMARS))

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("// Generated by scripts/tools/gen/syllable-machines.py from scripts/data/grammars/.\n")
        f.write("// Do not edit: change the grammar and re-run.\n")
        f.write("//\n")
        f.write("// Each table is a scanner. Walking it from a position and remembering the last\n")
        f.write("// accepting state gives the longest match, which is the rule these grammars are\n")
        f.write("// written against – several productions can match at one position and the longest\n")
        f.write("// one wins, not the first.\n\n")
        f.write("// No transition: the syllable ended before here.\n")
        f.write("pub(crate) const DEAD: u8 = 0xFF;\n")
        f.write("// No syllable type accepted in this state.\n")
        f.write("const NONE: u8 = 0xFF;\n\n")

        grammars = {name: parse_grammar(os.path.join(GRAMMARS, name + ".grammar")) for name in names}
        # Grammars that find the same syllable kinds share one enum, named for the one its table is built from.
        kinds_of = {}
        for name in sorted(names, key=lambda n: TABLE_OF.get(n) != n):
            kinds_of.setdefault(tuple(syllable_types(grammars[name][2])), name)
        for name in names:
            categories, definitions, syllables = grammars[name]
            width = alphabet_width(name, categories, grammars["use"][0])
            transitions, accepts = compile_machine(categories, definitions, syllables, width)
            before = len(transitions)
            small = minimize(transitions, accepts)
            if difference((transitions, accepts), small, len(transitions[0])) is not None:
                sys.exit("{}: minimizing changed the syllables it finds".format(name))
            transitions, accepts = small

            emit(f, name, kinds_of[tuple(syllable_types(syllables))], syllables, transitions, accepts)
            print("  {:<8} {} states (from {}), {} categories, {} rules, {} syllable types".format(
                name, len(transitions), before, len(transitions[0]),
                len(syllables), len(syllable_types(syllables))))

    print("wrote {}".format(OUT))

if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--compare":
        sys.exit(0 if compare(sys.argv[2], sys.argv[3]) else 1)
    if len(sys.argv) > 1:
        sys.stderr.write(
            "syllable-machines.py rewrites {} in place, or with --compare OLD NEW tells whether two grammars find the"
            " same syllables.\n".format(OUT)
        )
        sys.exit(2)
    main()
