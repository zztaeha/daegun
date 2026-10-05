#!/usr/bin/env python3

import os
import re
import sys

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

    def __init__(self, tokens, categories, definitions, expanding=()):
        self.tokens = tokens
        self.at = 0
        self.categories = categories
        self.definitions = definitions
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
            width = max(self.categories.values()) + 1
            return Alt([Sym(c) for c in range(width)]) if width > 1 else Sym(0)
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
                self.definitions[tok], self.categories, self.definitions, self.expanding + (tok,)
            )
        sys.exit("unknown symbol {!r}".format(tok))

def parse_expression(text, categories, definitions, expanding=()):
    return Parser(tokenize(text), categories, definitions, expanding).parse()

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

def compile_machine(categories, definitions, syllables):
    types = syllable_types(syllables)
    rule_type = [types.index(name) for name, _ in syllables]
    nfa = Nfa()
    start = nfa.new_state()
    accept_rule = {}

    for index, (_, body) in enumerate(syllables):
        branch = nfa.new_state()
        nfa.link(start, branch)
        end = nfa.build(parse_expression(body, categories, definitions), branch)
        accept_rule.setdefault(end, index)
        if accept_rule[end] > index:
            accept_rule[end] = index

    width = max(categories.values()) + 1
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

def check_equivalent(before, after, width, rounds=4000):
    import random
    rng = random.Random(0x5EED)
    for _ in range(rounds):
        length = rng.randrange(1, 12)
        text = [rng.randrange(width) for _ in range(length)]
        if run(*before, text) != run(*after, text):
            sys.exit("minimization changed the segmentation of {}".format(text))

def emit(f, name, categories, syllables, transitions, accepts):
    upper = name.upper()
    width = len(transitions[0])
    types = syllable_types(syllables)

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

    f.write("// `[state][category]` -> next state, or `DEAD`.\n")
    f.write("pub(crate) static {}_TRANSITIONS: [[u16; {}]; {}] = [\n".format(upper, width, len(transitions)))
    for row in transitions:
        f.write("    [{}],\n".format(", ".join("DEAD" if v == DEAD else str(v) for v in row)))
    f.write("];\n\n")

    f.write("// Which syllable type each state accepts, if any.\n")
    f.write("pub(crate) static {}_ACCEPTS: [u8; {}] = [\n    ".format(upper, len(accepts)))
    f.write(", ".join("NONE" if a is None else str(a) for a in accepts))
    f.write(",\n];\n\n")

    f.write("pub(crate) fn {}_accept(state: u16) -> Option<{}Syllable> {{\n".format(name, name.capitalize()))
    f.write("    {}Syllable::from_index(*{}_ACCEPTS.get(state as usize)?)\n}}\n\n".format(name.capitalize(), upper))

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
        f.write("pub(crate) const DEAD: u16 = 0xFFFF;\n")
        f.write("// No syllable type accepted in this state.\n")
        f.write("const NONE: u8 = 0xFF;\n\n")

        for name in names:
            path = os.path.join(GRAMMARS, name + ".grammar")
            categories, definitions, syllables = parse_grammar(path)
            transitions, accepts = compile_machine(categories, definitions, syllables)
            before = len(transitions)
            small = minimize(transitions, accepts)
            check_equivalent((transitions, accepts), small, len(transitions[0]))
            transitions, accepts = small

            emit(f, name, categories, syllables, transitions, accepts)
            print("  {:<8} {} states (from {}), {} categories, {} rules, {} syllable types".format(
                name, len(transitions), before, len(transitions[0]),
                len(syllables), len(syllable_types(syllables))))

    print("wrote {}".format(OUT))

if __name__ == "__main__":
    if len(sys.argv) > 1:
        sys.stderr.write(
            "gen-syllable-machines.py takes no arguments – it rewrites {} in place.\n".format(OUT)
        )
        sys.exit(2)
    main()
