#!/usr/bin/env python3
"""Development-only script (SPEC section 6): produces reference embeddings
for a fixed set of texts using the real sentence-transformers/BAAI
implementation of bge-small-en-v1.5, at the same pinned revision as
`cargo xtask fetch-model`. The output is committed and compared against the
Rust implementation by the `parity` feature test in crates/embed.

Usage (from the repo root):
    python3 -m venv .venv-reference-embeddings
    source .venv-reference-embeddings/bin/activate
    pip install sentence-transformers
    python3 xtask/reference_embeddings.py
"""

import json
from pathlib import Path

from sentence_transformers import SentenceTransformer

REVISION = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"

# Ten fixed (title, abstract) pairs covering short/long text and varied
# vocabulary, formatted exactly as crates/embed does: "{title}. {abstract}".
TEXTS = [
    ("Apparatus for manufacturing green bricks",
     "A device for forming green bricks from clay for the brick manufacturing industry, "
     "comprising a circulating conveyor carrying mould containers."),
    ("Method and system for placing a purchase order via a communications network",
     "A method for placing an order to purchase an item via the Internet, in which a client "
     "system sends purchaser information to a server system."),
    ("Method for node ranking in a linked database",
     "A method assigns importance ranks to nodes in a linked database by iteratively "
     "computing a rank based on the ranks of nodes that link to it."),
    ("Microbial fuel cell",
     "A fuel cell that uses microorganisms to catalyse the oxidation of organic matter, "
     "generating an electrical current at an anode."),
    ("A system and method for managing information between a server and a computerized device",
     "Information is synchronised between a server and a client device using a set of rules "
     "that determine which changes take precedence."),
    ("Gas discharge display device",
     "A display device using spacing elements between two substrates to maintain a uniform "
     "gap for gas discharge cells arranged in a matrix."),
    ("Mouse controller",
     "A hand-operated pointing device with a rotatable ball that transmits motion to sensors, "
     "generating signals proportional to the movement."),
    ("Developer comprising toner and carrier",
     "An electrophotographic developer comprising toner particles and carrier particles, "
     "the toner having a specified average degree of roundness."),
    ("Multi-depth display apparatus",
     "A display apparatus capable of presenting images at multiple perceived depths "
     "simultaneously, using a stack of transparent display layers."),
    ("Starting mechanism for an internal combustion engine",
     "A starting mechanism featuring a separate insertion process and starting process, "
     "reducing wear on the engagement gear during engine start."),
]


def main() -> None:
    # CPU only, matching crates/embed (candle_core::Device::Cpu) and SPEC
    # section 2's "CPU only; no GPU required" - also sidesteps a CUDA
    # build/driver mismatch that isn't relevant to what we're comparing.
    model = SentenceTransformer("BAAI/bge-small-en-v1.5", revision=REVISION, device="cpu")
    inputs = [f"{title}. {abstract}" for title, abstract in TEXTS]
    embeddings = model.encode(inputs, normalize_embeddings=True)

    out = {
        "model": "BAAI/bge-small-en-v1.5",
        "revision": REVISION,
        "cases": [
            {"text": text, "embedding": embedding.tolist()}
            for text, embedding in zip(inputs, embeddings)
        ],
    }

    out_path = Path(__file__).parent.parent / "crates" / "embed" / "tests" / "reference_embeddings.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(out, indent=2) + "\n")
    print(f"Wrote {len(out['cases'])} reference embeddings to {out_path}")


if __name__ == "__main__":
    main()
