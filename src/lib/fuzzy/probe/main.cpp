// Suite 0 rung 1: expose the C++ scorer's ranking so it can be diffed against
// the Rust port's. See docs/rust-engine/PLAN.md §8.1a.
//
// WHY THIS EXISTS AT ALL
//
// Suite 0 is specified (§8.1) as a differential harness over both engines, and
// it could not run: the C++ CLI has no command that emits ranked results, and
// figura/ipc.fig has no ranked-search method — its only query is fsQuery, over
// files. Adding that surface to the shipping engine is real work in a tree we
// are deleting, so it is scoped separately (§8.1a rung 2).
//
// What made a cheaper first rung possible is that `vicinae::fuzzy` is a
// header-only INTERFACE library with no Qt dependency: this file compiles and
// links with a bare `c++ -std=c++23 -Isrc/lib/fuzzy/include`, in about a
// second, with no CMake, no Qt, no server and no display. That is the whole
// reason this probe is affordable to run on every PR.
//
// WHY IT PARSES NOTHING
//
// It reads `id<TAB>text` lines on stdin and scores them. It deliberately does
// NOT read the corpus itself, because then two parsers would exist — this one
// and the Rust driver's — and a disagreement about which `Name=` line to take
// would show up as a scoring divergence. One parser, on the Rust side; this
// binary only scores strings it is handed.
//
// SCOPE, so a green diff is not over-read: this is the SCORER, not the engine's
// ranking. RootItemManager::searchGroupedByProvider wraps it in provider
// bucketing, a separate provider-name score, favourite and enabled filtering,
// and per-item frecency. Scorer parity is necessary for ranking parity and is
// not the same thing.
#include <algorithm>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

#include "fuzzy/fuzzy-searchable.hpp"

namespace {

struct Item {
  std::string id;
  std::string text;
};

struct Hit {
  const Item *item;
  int score;
  int quality;
};

} // namespace

int main(int argc, char **argv) {
  if (argc != 2) {
    std::cerr << "usage: vicinae-fuzzy-probe <query>   (items as id<TAB>text on stdin)\n";
    return 2;
  }

  std::vector<Item> items;
  std::string line;
  while (std::getline(std::cin, line)) {
    if (line.empty()) continue;
    auto const tab = line.find('\t');
    if (tab == std::string::npos) {
      std::cerr << "malformed input line, expected id<TAB>text: " << line << "\n";
      return 2;
    }
    items.push_back({line.substr(0, tab), line.substr(tab + 1)});
  }

  fuzzy::Query const query{std::string{argv[1]}};

  std::vector<Hit> hits;
  hits.reserve(items.size());
  for (auto const &item : items) {
    auto const m = fuzzy::scoreWeighted({{item.text, 1.0}}, query);
    if (m.accepted()) hits.push_back({&item, m.score, m.quality});
  }

  // stable_sort, and the input order is the caller's: the driver feeds items in
  // a fixed order so that ties resolve identically on both sides. An unstable
  // sort here would make tie order an artifact of the algorithm rather than
  // something the diff can hold constant.
  std::stable_sort(hits.begin(), hits.end(), [](Hit const &a, Hit const &b) { return a.score > b.score; });

  for (auto const &h : hits) {
    std::cout << h.item->id << '\t' << h.score << '\t' << h.quality << '\n';
  }
  return 0;
}
