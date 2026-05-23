#!/usr/bin/env python3
"""gen_figures.py — gera as figuras dos slides (matplotlib).

Saidas em img/:
  cosine_geometry.png  — a intuicao do cosseno: angulo entre setas (3 casos)
  dot_steps.png        — produto interno passo a passo (exemplo 2D minusculo)
  zipf.png             — a lei de Zipf com dados reais (le ../zipf.csv)
  sparsity.png         — embedding esparso: quase tudo zero

Uso:  python3 gen_figures.py
"""
import os
import math
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

os.makedirs("img", exist_ok=True)
BLUE, RED, GRAY = "#2471a3", "#c0392b", "#7f8c8d"


def _draw_pair(ax, v, w, title, note):
    for vec, col in [(v, BLUE), (w, RED)]:
        ax.annotate("", xy=vec, xytext=(0, 0),
                    arrowprops=dict(arrowstyle="-|>", color=col, lw=2.5))
    ax.text(v[0], v[1] + 0.15, "documento", color=BLUE, fontsize=11, ha="center")
    ax.text(w[0], w[1] + 0.15, "busca", color=RED, fontsize=11, ha="center")
    cos = (v[0]*w[0] + v[1]*w[1]) / (math.hypot(*v) * math.hypot(*w))
    ang = math.degrees(math.acos(max(-1, min(1, cos))))
    ax.set_title(title, fontsize=13, weight="bold")
    ax.text(0.5, -0.9, f"angulo ~ {ang:.0f}°   →   cosseno = {cos:.2f}\n{note}",
            transform=ax.transData, ha="center", fontsize=11)
    ax.set_xlim(-0.5, 4.2); ax.set_ylim(-1.4, 4.2)
    ax.axhline(0, color="#ddd", lw=1); ax.axvline(0, color="#ddd", lw=1)
    ax.set_aspect("equal"); ax.set_xticks([]); ax.set_yticks([])


def cosine_geometry():
    fig, axs = plt.subplots(1, 3, figsize=(15, 5.2))
    _draw_pair(axs[0], (3, 2.6), (2.6, 3), "MESMA DIRECAO", "mesmo assunto → cosseno perto de 1")
    _draw_pair(axs[1], (3.6, 0), (0, 3.6), "PERPENDICULAR", "nada a ver → cosseno = 0")
    _draw_pair(axs[2], (3, 1), (3.6, 1.2), "QUASE IGUAIS", "praticamente o mesmo → ~1")
    fig.suptitle("Cosseno = o quanto duas setas APONTAM pro mesmo lado",
                 fontsize=15, weight="bold")
    plt.tight_layout(rect=[0, 0, 1, 0.95])
    plt.savefig("img/cosine_geometry.png", dpi=120); plt.close()


def dot_steps():
    """Exemplo 2D minusculo: doc=[3,1], busca=[2,0] — conta passo a passo."""
    fig, ax = plt.subplots(figsize=(11, 5.5))
    ax.axis("off")
    lines = [
        ("doc   = [ 3 , 1 ]      (3 vezes a silaba 'ca', 1 vez 'sa')", "#111"),
        ("busca = [ 2 , 0 ]      (2 vezes 'ca', 0 vezes 'sa')", "#111"),
        ("", "#111"),
        ("1) produto interno  = 3×2 + 1×0 = 6", BLUE),
        ("2) tamanho do doc   = √(3² + 1²) = √10 ≈ 3.16", GRAY),
        ("3) tamanho da busca = √(2² + 0²) = √4  = 2.00", GRAY),
        ("", "#111"),
        ("cosseno = 6 / (3.16 × 2.00) = 6 / 6.32 ≈ 0.95", RED),
        ("", "#111"),
        ("0.95 perto de 1  →  muito parecidos ✓", "#1e8449"),
    ]
    y = 0.93
    for txt, col in lines:
        ax.text(0.04, y, txt, fontsize=15, color=col, family="monospace",
                weight="bold" if col in (BLUE, RED, "#1e8449") else "normal")
        y -= 0.095
    ax.set_title("Cosseno na mão — sem trigonometria, só multiplicar e somar",
                 fontsize=15, weight="bold", loc="left")
    plt.tight_layout()
    plt.savefig("img/dot_steps.png", dpi=120); plt.close()


def zipf():
    ranks, counts = [], []
    try:
        with open("../zipf.csv", encoding="utf-8") as f:
            next(f)
            for line in f:
                r, _tok, c, _freq = line.rstrip("\n").split(",")
                ranks.append(int(r)); counts.append(int(c))
    except FileNotFoundError:
        print("  (zipf.csv nao encontrado — rode antes: python3 ../01_tokenizer_zipf.py)")
        return
    fig, (a1, a2) = plt.subplots(1, 2, figsize=(14, 5.2))
    a1.bar(ranks[:25], counts[:25], color=BLUE)
    a1.set_title("Os 25 tokens mais comuns", weight="bold")
    a1.set_xlabel("rank (1 = mais comum)"); a1.set_ylabel("ocorrencias")
    a2.loglog(ranks, counts, color=RED, lw=2)
    a2.set_title("A mesma curva em escala log-log\n(quase uma reta = lei de Zipf)", weight="bold")
    a2.set_xlabel("rank (log)"); a2.set_ylabel("ocorrencias (log)"); a2.grid(alpha=0.3)
    fig.suptitle("Lei de Zipf: pouquíssimos tokens fazem quase todo o texto",
                 fontsize=15, weight="bold")
    plt.tight_layout(rect=[0, 0, 1, 0.93])
    plt.savefig("img/zipf.png", dpi=120); plt.close()


def sparsity():
    fig, ax = plt.subplots(figsize=(13, 3.2))
    import random
    random.seed(7)
    n = 120
    vals = [0] * n
    for i in random.sample(range(n), 14):
        vals[i] = random.randint(1, 9)
    ax.bar(range(n), vals, color=[BLUE if v else "#e5e7eb" for v in vals], width=0.9)
    ax.set_title("Embedding ESPARSO: das ~2000 dimensoes, só uma mãozinha é ≠ 0",
                 fontsize=14, weight="bold")
    ax.set_xlabel("dimensao (cada uma é uma sílaba do vocabulário)")
    ax.set_yticks([]); ax.set_xticks([])
    plt.tight_layout()
    plt.savefig("img/sparsity.png", dpi=120); plt.close()


if __name__ == "__main__":
    cosine_geometry(); dot_steps(); zipf(); sparsity()
    print("figuras geradas em img/: cosine_geometry, dot_steps, zipf, sparsity")
