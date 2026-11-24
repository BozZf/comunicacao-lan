#!/usr/bin/env python3
"""
Gerador do Relatório de Auditoria de Segurança — comunicacao-lan.

Gera docs/security-audit/relatorio-auditoria-seguranca.pdf em formato A4 (margens ~2 cm),
com gráficos em 300 DPI (matplotlib) e layout estruturado (reportlab).

Uso em ambiente isolado:
    python3 -m venv .venv
    .venv/bin/pip install -r docs/security-audit/requirements.txt
    .venv/bin/python docs/security-audit/generate_report.py
"""

from __future__ import annotations

import io
import math
import os
from pathlib import Path
from xml.sax.saxutils import escape

import matplotlib

matplotlib.use("Agg")
import matplotlib.patches as mpatches
import matplotlib.pyplot as plt
from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import cm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas
from reportlab.platypus import (
    Flowable,
    Image,
    KeepTogether,
    PageBreak,
    Paragraph,
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)

# ---------------------------------------------------------------------------
# Paleta oficial solicitada
# ---------------------------------------------------------------------------
HEX_CRITICA = "#B91C1C"
HEX_ALTA = "#EA580C"
HEX_MEDIA = "#D97706"
HEX_BAIXA = "#2563EB"
HEX_INFO = "#64748B"
HEX_FORTE = "#059669"

HEX_SLATE_900 = "#0F172A"
HEX_SLATE_800 = "#1E293B"
HEX_SLATE_700 = "#334155"
HEX_SLATE_500 = "#64748B"
HEX_SLATE_300 = "#CBD5E1"
HEX_SLATE_200 = "#E2E8F0"
HEX_SLATE_100 = "#F1F5F9"
HEX_SLATE_50 = "#F8FAFC"

C_CRITICA = colors.HexColor(HEX_CRITICA)
C_ALTA = colors.HexColor(HEX_ALTA)
C_MEDIA = colors.HexColor(HEX_MEDIA)
C_BAIXA = colors.HexColor(HEX_BAIXA)
C_INFO = colors.HexColor(HEX_INFO)
C_FORTE = colors.HexColor(HEX_FORTE)

C_SLATE_900 = colors.HexColor(HEX_SLATE_900)
C_SLATE_800 = colors.HexColor(HEX_SLATE_800)
C_SLATE_700 = colors.HexColor(HEX_SLATE_700)
C_SLATE_500 = colors.HexColor(HEX_SLATE_500)
C_SLATE_300 = colors.HexColor(HEX_SLATE_300)
C_SLATE_200 = colors.HexColor(HEX_SLATE_200)
C_SLATE_100 = colors.HexColor(HEX_SLATE_100)
C_SLATE_50 = colors.HexColor(HEX_SLATE_50)

SEVERITY_COLORS = {
    "Crítica": HEX_CRITICA,
    "Alta": HEX_ALTA,
    "Média": HEX_MEDIA,
    "Baixa": HEX_BAIXA,
    "Informativa": HEX_INFO,
    "Ponto Forte": HEX_FORTE,
}

REPORT_TITLE = "Relatório de Auditoria de Segurança — comunicacao-lan"
AUDIT_DATE = "24 de novembro de 2026"
REPO_NAME = "BozZf/comunicacao-lan"
COMMIT_HASH = "100bfc9"


# ---------------------------------------------------------------------------
# Registro de fontes TrueType (portável Linux / fallback padrão)
# ---------------------------------------------------------------------------
def register_fonts() -> tuple[str, str, str, str]:
    """Registra fontes sans-serif e monoespaçadas com suporte completo a pt-BR."""
    candidates = [
        (
            "Sans",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        ),
        (
            "Sans",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ),
    ]
    for family, reg, bold, italic, mono in candidates:
        if all(os.path.exists(p) for p in (reg, bold, italic, mono)):
            pdfmetrics.registerFont(TTFont(f"{family}", reg))
            pdfmetrics.registerFont(TTFont(f"{family}-Bold", bold))
            pdfmetrics.registerFont(TTFont(f"{family}-Italic", italic))
            pdfmetrics.registerFont(TTFont("Mono", mono))
            pdfmetrics.registerFontFamily(
                family,
                normal=family,
                bold=f"{family}-Bold",
                italic=f"{family}-Italic",
                boldItalic=f"{family}-Bold",
            )
            return family, f"{family}-Bold", f"{family}-Italic", "Mono"
    return "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Courier"


FONT_REG, FONT_BOLD, FONT_ITALIC, FONT_MONO = register_fonts()


# ---------------------------------------------------------------------------
# Flowables customizados
# ---------------------------------------------------------------------------
class SeverityChip(Flowable):
    """Chip colorido arredondado com ajuste automático de fonte para evitar corte."""

    def __init__(
        self,
        label: str,
        hex_color: str,
        width: float = 2.15 * cm,
        height: float = 0.50 * cm,
        font_size: float = 7.1,
    ):
        super().__init__()
        self.label = label.upper()
        self.bg = colors.HexColor(hex_color)
        self.width = width
        self.height = height
        # Garante que o texto sempre caiba confortavelmente dentro do chip
        max_text_w = self.width - 8.0
        fs = font_size
        while fs > 5.5 and pdfmetrics.stringWidth(self.label, FONT_BOLD, fs) > max_text_w:
            fs -= 0.25
        self.font_size = fs

    def draw(self) -> None:
        c = self.canv
        c.saveState()
        c.setFillColor(self.bg)
        c.roundRect(0, 0, self.width, self.height, radius=3.5, stroke=0, fill=1)
        c.setFillColor(colors.white)
        c.setFont(FONT_BOLD, self.font_size)
        c.drawCentredString(self.width / 2.0, (self.height - self.font_size) / 2.0 + 1.4, self.label)
        c.restoreState()


class SectionHeader(Flowable):
    """Cabeçalho de seção com barra lateral colorida e numeração."""

    def __init__(self, number: str, title: str, width: float, accent_hex: str = HEX_SLATE_900):
        super().__init__()
        self.number = number
        self.title = title
        self.width = width
        self.height = 0.78 * cm
        self.accent = colors.HexColor(accent_hex)

    def draw(self) -> None:
        c = self.canv
        c.saveState()
        c.setFillColor(self.accent)
        c.roundRect(0, 0.14 * cm, 0.14 * cm, 0.52 * cm, radius=1.5, stroke=0, fill=1)
        c.setFont(FONT_BOLD, 13.0)
        c.setFillColor(self.accent)
        c.drawString(0.32 * cm, 0.24 * cm, f"{self.number}  {self.title}")
        c.setStrokeColor(C_SLATE_200)
        c.setLineWidth(0.75)
        c.line(0, 0.02 * cm, self.width, 0.02 * cm)
        c.restoreState()


# ---------------------------------------------------------------------------
# Desenho de fundo da capa (antes dos flowables) e rodapé/cabeçalho "Página X de Y"
# ---------------------------------------------------------------------------
COVER_BANNER_HEIGHT = 5.75 * cm


def draw_cover_background(c: canvas.Canvas, _doc) -> None:
    """Desenha o banner escuro da capa ANTES dos flowables para não cobrir o título."""
    width, height = A4
    c.saveState()
    c.setFillColor(C_SLATE_900)
    c.rect(0, height - COVER_BANNER_HEIGHT, width, COVER_BANNER_HEIGHT, stroke=0, fill=1)
    bar_y = height - COVER_BANNER_HEIGHT
    segments = [HEX_FORTE, HEX_BAIXA, HEX_MEDIA, HEX_ALTA, HEX_CRITICA]
    seg_w = width / len(segments)
    for idx, hx in enumerate(segments):
        c.setFillColor(colors.HexColor(hx))
        c.rect(idx * seg_w, bar_y - 0.13 * cm, seg_w + 0.5, 0.13 * cm, stroke=0, fill=1)
    c.restoreState()


class AuditCanvas(canvas.Canvas):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._saved_page_states = []

    def showPage(self) -> None:
        self._saved_page_states.append(dict(self.__dict__))
        self._startPage()

    def save(self) -> None:
        total_pages = len(self._saved_page_states)
        for state in self._saved_page_states:
            self.__dict__.update(state)
            self._draw_chrome(total_pages)
            super().showPage()
        super().save()

    def _draw_chrome(self, total_pages: int) -> None:
        page_num = self._pageNumber
        width, height = A4
        margin = 2.0 * cm

        self.saveState()
        if page_num == 1:
            # Apenas rodapé discreto na capa (o banner já foi desenhado em onFirstPage)
            self.setStrokeColor(C_SLATE_200)
            self.setLineWidth(0.6)
            self.line(margin, 1.38 * cm, width - margin, 1.38 * cm)
            self.setFont(FONT_REG, 7.9)
            self.setFillColor(C_SLATE_500)
            self.drawString(margin, 0.92 * cm, f"{REPORT_TITLE}  |  {REPO_NAME} ({COMMIT_HASH})")
            self.setFont(FONT_BOLD, 7.9)
            self.setFillColor(C_SLATE_700)
            self.drawRightString(width - margin, 0.92 * cm, f"Página {page_num} de {total_pages}")
        else:
            # Cabeçalho das páginas de conteúdo
            header_y = height - 1.30 * cm
            self.setFont(FONT_BOLD, 8.1)
            self.setFillColor(C_SLATE_700)
            self.drawString(margin, header_y, REPORT_TITLE)
            self.setFont(FONT_REG, 7.9)
            self.setFillColor(C_SLATE_500)
            self.drawRightString(width - margin, header_y, AUDIT_DATE)
            self.setStrokeColor(C_SLATE_200)
            self.setLineWidth(0.65)
            self.line(margin, header_y - 0.18 * cm, width - margin, header_y - 0.18 * cm)

            # Rodapé
            footer_y = 0.92 * cm
            self.setStrokeColor(C_SLATE_200)
            self.setLineWidth(0.65)
            self.line(margin, footer_y + 0.42 * cm, width - margin, footer_y + 0.42 * cm)
            self.setFont(FONT_REG, 7.8)
            self.setFillColor(C_SLATE_500)
            self.drawString(
                margin,
                footer_y,
                f"Auditoria de Segurança Estática · {REPO_NAME} · Commit {COMMIT_HASH}",
            )
            self.setFont(FONT_BOLD, 7.9)
            self.setFillColor(C_SLATE_700)
            self.drawRightString(width - margin, footer_y, f"Página {page_num} de {total_pages}")

        self.restoreState()


# ---------------------------------------------------------------------------
# Geração dos gráficos (matplotlib em 300 DPI em memória)
# ---------------------------------------------------------------------------
def build_donut_chart() -> io.BytesIO:
    """Gráfico de rosca por severidade com paleta oficial."""
    plt.rcParams["font.sans-serif"] = ["Liberation Sans", "DejaVu Sans", "Arial"]
    fig, ax = plt.subplots(figsize=(4.4, 3.15), dpi=300)
    fig.patch.set_facecolor("white")

    active_values = [2, 3, 1]
    active_colors = [HEX_MEDIA, HEX_BAIXA, HEX_INFO]

    wedges, _ = ax.pie(
        active_values,
        colors=active_colors,
        startangle=135,
        counterclock=False,
        wedgeprops={"width": 0.36, "edgecolor": "white", "linewidth": 2.4},
    )

    for wedge, val in zip(wedges, active_values):
        ang = (wedge.theta2 + wedge.theta1) / 2.0
        x = 0.82 * math.cos(math.radians(ang))
        y = 0.82 * math.sin(math.radians(ang))
        ax.text(
            x,
            y,
            str(val),
            ha="center",
            va="center",
            color="white",
            fontsize=11,
            fontweight="bold",
        )

    ax.text(0, 0.08, "6", ha="center", va="center", fontsize=23, fontweight="bold", color=HEX_SLATE_900)
    ax.text(0, -0.17, "Achados\nVerificados", ha="center", va="center", fontsize=8.4, color=HEX_SLATE_500)

    ax.set_title(
        "Distribuição por Severidade",
        fontsize=10.6,
        fontweight="bold",
        color=HEX_SLATE_900,
        pad=10,
    )

    legend_entries = [
        mpatches.Patch(color=HEX_CRITICA, label="Crítica: 0"),
        mpatches.Patch(color=HEX_ALTA, label="Alta: 0"),
        mpatches.Patch(color=HEX_MEDIA, label="Média: 2"),
        mpatches.Patch(color=HEX_BAIXA, label="Baixa: 3"),
        mpatches.Patch(color=HEX_INFO, label="Informativa: 1"),
    ]
    ax.legend(
        handles=legend_entries,
        loc="center left",
        bbox_to_anchor=(0.95, 0.5),
        frameon=False,
        fontsize=8.3,
        handlelength=1.1,
        handleheight=1.1,
        labelspacing=0.65,
    )

    ax.set(aspect="equal")
    plt.subplots_adjust(left=0.02, right=0.68, top=0.86, bottom=0.05)

    buf = io.BytesIO()
    fig.savefig(buf, format="png", dpi=300, facecolor="white")
    plt.close(fig)
    buf.seek(0)
    return buf


def build_category_bar_chart() -> io.BytesIO:
    """Gráfico de barras empilhadas por categoria usando a paleta oficial."""
    plt.rcParams["font.sans-serif"] = ["Liberation Sans", "DejaVu Sans", "Arial"]
    fig, ax = plt.subplots(figsize=(4.6, 3.15), dpi=300)
    fig.patch.set_facecolor("white")
    ax.set_facecolor("white")

    categories = [
        "1. Banco sem\nTranca*",
        "2. Permissão\nno Navegador*",
        "3. IDOR*",
        "4. Chaves\nExpostas",
        "5. Inputs /\nXSS*",
    ]
    y_pos = list(range(len(categories)))

    media = [1, 1, 0, 0, 0]
    baixa = [0, 0, 1, 1, 1]
    info = [0, 0, 0, 1, 0]

    bar_h = 0.52
    ax.barh(y_pos, media, height=bar_h, color=HEX_MEDIA, label="Média", edgecolor="white", linewidth=1.0)
    ax.barh(
        y_pos,
        baixa,
        left=media,
        height=bar_h,
        color=HEX_BAIXA,
        label="Baixa",
        edgecolor="white",
        linewidth=1.0,
    )
    left_info = [m + b for m, b in zip(media, baixa)]
    ax.barh(
        y_pos,
        info,
        left=left_info,
        height=bar_h,
        color=HEX_INFO,
        label="Informativa",
        edgecolor="white",
        linewidth=1.0,
    )

    totals = [m + b + i for m, b, i in zip(media, baixa, info)]
    for y, tot in zip(y_pos, totals):
        ax.text(
            tot + 0.08,
            y,
            f"{tot}",
            va="center",
            ha="left",
            fontsize=9.2,
            fontweight="bold",
            color=HEX_SLATE_800,
        )

    ax.set_yticks(y_pos)
    ax.set_yticklabels(categories, fontsize=8.0, color=HEX_SLATE_700)
    ax.invert_yaxis()
    ax.set_xlim(0, 2.55)
    ax.set_xticks([0, 1, 2])
    ax.tick_params(axis="x", labelsize=8.2, colors=HEX_SLATE_500)
    ax.xaxis.grid(True, linestyle="--", alpha=0.45, color=HEX_SLATE_300)
    ax.set_axisbelow(True)

    for spine in ("top", "right"):
        ax.spines[spine].set_visible(False)
    ax.spines["left"].set_color(HEX_SLATE_300)
    ax.spines["bottom"].set_color(HEX_SLATE_300)

    ax.set_title(
        "Achados por Categoria",
        fontsize=10.6,
        fontweight="bold",
        color=HEX_SLATE_900,
        pad=10,
    )
    ax.set_xlabel("Quantidade de achados", fontsize=8.0, color=HEX_SLATE_500, labelpad=4)

    ax.legend(
        loc="lower right",
        frameon=True,
        facecolor=HEX_SLATE_50,
        edgecolor=HEX_SLATE_200,
        fontsize=7.6,
    )

    fig.text(
        0.02,
        0.015,
        "* Categorias adaptadas à stack Rust (sem BD, servidor HTTP ou frontend web).",
        fontsize=6.8,
        color=HEX_SLATE_500,
        style="italic",
    )

    plt.subplots_adjust(left=0.31, right=0.95, top=0.86, bottom=0.19)

    buf = io.BytesIO()
    fig.savefig(buf, format="png", dpi=300, facecolor="white")
    plt.close(fig)
    buf.seek(0)
    return buf


# ---------------------------------------------------------------------------
# Estilos tipográficos
# ---------------------------------------------------------------------------
def build_styles() -> dict[str, ParagraphStyle]:
    base = getSampleStyleSheet()
    return {
        "cover_kicker": ParagraphStyle(
            "cover_kicker",
            parent=base["Normal"],
            fontName=FONT_BOLD,
            fontSize=8.6,
            leading=11,
            textColor=colors.HexColor("#34D399"),
            spaceAfter=4,
        ),
        "cover_title": ParagraphStyle(
            "cover_title",
            parent=base["Title"],
            fontName=FONT_BOLD,
            fontSize=19.5,
            leading=23.5,
            textColor=colors.white,
            alignment=TA_LEFT,
            spaceAfter=5,
        ),
        "cover_sub": ParagraphStyle(
            "cover_sub",
            parent=base["Normal"],
            fontName=FONT_REG,
            fontSize=9.6,
            leading=13.5,
            textColor=colors.HexColor("#CBD5E1"),
            alignment=TA_LEFT,
        ),
        "h2": ParagraphStyle(
            "h2",
            parent=base["Heading2"],
            fontName=FONT_BOLD,
            fontSize=10.6,
            leading=13.8,
            textColor=C_SLATE_900,
            spaceBefore=7,
            spaceAfter=3.5,
        ),
        "h3": ParagraphStyle(
            "h3",
            parent=base["Heading3"],
            fontName=FONT_BOLD,
            fontSize=9.4,
            leading=12.5,
            textColor=C_SLATE_800,
            spaceBefore=5,
            spaceAfter=3,
        ),
        "body": ParagraphStyle(
            "body",
            parent=base["Normal"],
            fontName=FONT_REG,
            fontSize=8.8,
            leading=12.6,
            textColor=C_SLATE_700,
            alignment=TA_JUSTIFY,
            spaceAfter=4,
        ),
        "body_left": ParagraphStyle(
            "body_left",
            parent=base["Normal"],
            fontName=FONT_REG,
            fontSize=8.6,
            leading=12.2,
            textColor=C_SLATE_700,
            alignment=TA_LEFT,
        ),
        "cell": ParagraphStyle(
            "cell",
            parent=base["Normal"],
            fontName=FONT_REG,
            fontSize=8.0,
            leading=11.0,
            textColor=C_SLATE_700,
            alignment=TA_LEFT,
        ),
        "cell_compact": ParagraphStyle(
            "cell_compact",
            parent=base["Normal"],
            fontName=FONT_REG,
            fontSize=7.85,
            leading=10.5,
            textColor=C_SLATE_700,
            alignment=TA_LEFT,
        ),
        "cell_bold": ParagraphStyle(
            "cell_bold",
            parent=base["Normal"],
            fontName=FONT_BOLD,
            fontSize=8.0,
            leading=11.0,
            textColor=C_SLATE_900,
            alignment=TA_LEFT,
        ),
        "cell_mono": ParagraphStyle(
            "cell_mono",
            parent=base["Normal"],
            fontName=FONT_MONO,
            fontSize=7.25,
            leading=10.0,
            textColor=C_SLATE_900,
            alignment=TA_LEFT,
        ),
        "th": ParagraphStyle(
            "th",
            parent=base["Normal"],
            fontName=FONT_BOLD,
            fontSize=8.0,
            leading=10.2,
            textColor=colors.white,
            alignment=TA_LEFT,
        ),
        "issue_md": ParagraphStyle(
            "issue_md",
            parent=base["Normal"],
            fontName=FONT_MONO,
            fontSize=7.05,
            leading=9.6,
            textColor=C_SLATE_900,
            alignment=TA_LEFT,
        ),
        "kpi_val": ParagraphStyle(
            "kpi_val",
            parent=base["Normal"],
            fontName=FONT_BOLD,
            fontSize=17,
            leading=20,
            alignment=TA_CENTER,
        ),
        "kpi_lbl": ParagraphStyle(
            "kpi_lbl",
            parent=base["Normal"],
            fontName=FONT_BOLD,
            fontSize=7.1,
            leading=9.2,
            textColor=C_SLATE_500,
            alignment=TA_CENTER,
        ),
    }


# ---------------------------------------------------------------------------
# Helpers de componentes
# ---------------------------------------------------------------------------
def make_callout(
    title: str,
    body_html: str,
    accent_hex: str,
    bg_hex: str,
    styles: dict[str, ParagraphStyle],
    content_width: float,
) -> Table:
    inner = Paragraph(
        f"<font name='{FONT_BOLD}' color='{accent_hex}'>{escape(title)}</font><br/>{body_html}",
        styles["body_left"],
    )
    tbl = Table([[inner]], colWidths=[content_width])
    tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, -1), colors.HexColor(bg_hex)),
                ("LINEBEFORE", (0, 0), (0, -1), 3.2, colors.HexColor(accent_hex)),
                ("BOX", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("TOPPADDING", (0, 0), (-1, -1), 6.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 6.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 10),
                ("RIGHTPADDING", (0, 0), (-1, -1), 9),
            ]
        )
    )
    return tbl


def make_findings_table(
    rows: list[dict],
    styles: dict[str, ParagraphStyle],
    content_width: float,
) -> Table:
    col_widths = [2.35 * cm, 3.45 * cm, content_width - 5.80 * cm]
    data = [
        [
            Paragraph("Severidade", styles["th"]),
            Paragraph("Arquivo:linha", styles["th"]),
            Paragraph("Descrição, Evidência e Explorabilidade", styles["th"]),
        ]
    ]

    for item in rows:
        sev = item["severity"]
        chip = SeverityChip(sev, SEVERITY_COLORS[sev], width=2.05 * cm, height=0.48 * cm)
        loc = Paragraph(item["location_html"], styles["cell_mono"])

        desc_parts = [
            f"<font name='{FONT_BOLD}' color='{HEX_SLATE_900}'>{escape(item['id'])} — {escape(item['title'])}</font>",
            f"<br/>{item['description_html']}",
            f"<br/><font name='{FONT_BOLD}' color='{HEX_SLATE_800}'>Por que é explorável:</font> {item['exploitability_html']}",
            f"<br/><font name='{FONT_BOLD}' color='{HEX_SLATE_800}'>Condição:</font> {item['condition_html']}",
        ]
        if item.get("code"):
            escaped_code = "<br/>".join(
                escape(line).replace(" ", "&nbsp;") for line in item["code"].strip("\n").splitlines()
            )
            desc_parts.append(
                f"<br/><font name='{FONT_BOLD}' color='{HEX_SLATE_800}'>Trecho verificado:</font>"
                f"<br/><font name='{FONT_MONO}' size='7.0' color='{HEX_SLATE_900}'>{escaped_code}</font>"
            )

        data.append([chip, loc, Paragraph("".join(desc_parts), styles["cell"])])

    tbl = Table(data, colWidths=col_widths, repeatRows=1)
    tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_SLATE_900),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, C_SLATE_50]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("TOPPADDING", (0, 0), (-1, -1), 5.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 5.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 5.5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 5.5),
            ]
        )
    )
    return tbl


def make_issue_block(issue_markdown: str, styles: dict[str, ParagraphStyle], content_width: float) -> Table:
    """Bloco monoespaçado copiável com quebra segura para issues GitHub."""
    rendered_lines = []
    for raw_line in issue_markdown.strip("\n").splitlines():
        if not raw_line:
            rendered_lines.append("&nbsp;")
        else:
            rendered_lines.append(escape(raw_line).replace("  ", "&nbsp;&nbsp;"))
    p = Paragraph("<br/>".join(rendered_lines), styles["issue_md"])
    tbl = Table([[p]], colWidths=[content_width])
    tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, -1), C_SLATE_50),
                ("BOX", (0, 0), (-1, -1), 0.75, C_SLATE_300),
                ("LINEBEFORE", (0, 0), (0, -1), 3.0, C_SLATE_800),
                ("TOPPADDING", (0, 0), (-1, -1), 7),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 7),
                ("LEFTPADDING", (0, 0), (-1, -1), 8.5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 8.5),
            ]
        )
    )
    return tbl


# ---------------------------------------------------------------------------
# Conteúdo estruturado (100% verificado no código real)
# ---------------------------------------------------------------------------
FINDINGS_BY_CATEGORY = [
    {
        "cat_num": 1,
        "cat_title": "Categoria 1 — Banco sem Tranca (Isolamento de Inquilino / Dono)",
        "applicability": (
            "<b>Aplicabilidade à stack:</b> <i>Parcialmente aplicável</i>. O projeto não possui banco de dados, "
            "ORM, RLS ou API HTTP. O único mecanismo de isolamento atual está em memória na struct "
            "<font name='Mono'>Station</font> (<font name='Mono'>src/lib.rs:40-46</font>), que isola "
            "<font name='Mono'>orders: HashMap&lt;String, Order&gt;</font> por <font name='Mono'>machine_id</font>."
        ),
        "findings": [
            {
                "id": "F-01",
                "severity": "Média",
                "location_html": "src/lib.rs:32-46<br/>src/lib.rs:65-84<br/>src/lib.rs:88-112",
                "title": "Isolamento e idempotência de charge_id restritos a uma única instância Station em memória",
                "description_html": (
                    "Cada instância de <font name='Mono'>Station</font> mantém seu mapa privado "
                    "<font name='Mono'>orders: HashMap&lt;String, Order&gt;</font> exclusivamente em RAM, e "
                    "<font name='Mono'>create_order</font> checa duplicidade apenas em <font name='Mono'>self.orders</font>. "
                    "Não há repositório multi-estação nem unicidade global de <font name='Mono'>charge_id</font> entre máquinas."
                ),
                "exploitability_html": (
                    "Duas instâncias distintas (<font name='Mono'>PC-01</font> e <font name='Mono'>PC-02</font>) aceitam "
                    "registrar e confirmar o mesmo <font name='Mono'>charge_id</font> sem disparar "
                    "<font name='Mono'>Error::DuplicateOrder</font>. Além disso, reinicializações do processo limpam "
                    "<font name='Mono'>orders</font>, permitindo replay de confirmações já creditadas."
                ),
                "condition_html": (
                    "Explorável quando a crate for integrada a um servidor multi-estação sem persistência transacional "
                    "impondo <font name='Mono'>UNIQUE(charge_id)</font> global (reconhecido em "
                    "<font name='Mono'>README.md:10, 24, 53-54</font>)."
                ),
                "code": (
                    "40: pub struct Station {\n"
                    "41:     machine_id: String,\n"
                    "42:     orders: HashMap<String, Order>,\n"
                    "75:     if self.orders.contains_key(&charge_id) {\n"
                    "76:         return Err(Error::DuplicateOrder);\n"
                    "77:     }"
                ),
            }
        ],
    },
    {
        "cat_num": 2,
        "cat_title": "Categoria 2 — Permissão Definida no Navegador (Fronteira de Privilégio)",
        "applicability": (
            "<b>Aplicabilidade à stack:</b> <i>Não aplicável na forma literal (N/A para frontend/navegador)</i> — "
            "não existem interface web, gates de papel (<font name='Mono'>isAdmin</font>, <font name='Mono'>canEdit</font>) "
            "ou endpoints HTTP. Adaptamos a revisão à fronteira de privilégio da API pública Rust."
        ),
        "findings": [
            {
                "id": "F-02",
                "severity": "Média",
                "location_html": "src/lib.rs:65-70<br/>src/lib.rs:88-112<br/>src/lib.rs:153-158",
                "title": "Operação privilegiada confirm_payment e relógio now expostos via primitivos sem tipo selado",
                "description_html": (
                    "<font name='Mono'>Station::confirm_payment</font> e <font name='Mono'>Station::create_order</font> "
                    "são <font name='Mono'>pub fn</font> aceitando primitivos brutos (<font name='Mono'>&amp;str</font>, "
                    "<font name='Mono'>u64</font>). A restrição de que somente o servidor após validar o provedor Pix "
                    "pode chamar <font name='Mono'>confirm_payment</font> existe apenas em comentários "
                    "(<font name='Mono'>src/lib.rs:1-3, 86-87</font>; <font name='Mono'>README.md:30-33</font>)."
                ),
                "exploitability_html": (
                    "Sem tipos de capacidade selados (ex.: <font name='Mono'>VerifiedPayment</font>, "
                    "<font name='Mono'>TrustedNow</font>), qualquer futuro handler HTTP, camada IPC ou UI no workspace "
                    "pode invocar <font name='Mono'>confirm_payment</font> diretamente ou fornecer "
                    "<font name='Mono'>now</font> manipulado sem barreira em tempo de compilação."
                ),
                "condition_html": (
                    "Explorável quando camadas futuras compartilharem a crate sem encapsular a operação em um adaptador "
                    "autenticado; atualmente não há servidor exposto."
                ),
                "code": (
                    "88: pub fn confirm_payment(\n"
                    "89:     &mut self, charge_id: &str, machine_id: &str,\n"
                    "92:     amount_cents: u64, now: u64,\n"
                    "94: ) -> Result<Vec<Event>, Error>"
                ),
            }
        ],
    },
    {
        "cat_num": 3,
        "cat_title": "Categoria 3 — IDOR (Referência Direta Insegura a Objeto)",
        "applicability": (
            "<b>Aplicabilidade à stack:</b> <i>Não aplicável a rotas HTTP (inexistentes)</i>. Auditados "
            "sistematicamente <b>todos os 5 métodos públicos</b> de <font name='Mono'>Station</font>. A validação de "
            "posse (<font name='Mono'>machine_id</font>) em <font name='Mono'>confirm_payment</font> "
            "(<font name='Mono'>src/lib.rs:95-98</font>) está <b>correta</b>; apontamos falha de canonização de IDs."
        ),
        "findings": [
            {
                "id": "F-03",
                "severity": "Baixa",
                "location_html": "src/lib.rs:49-55<br/>src/lib.rs:65-84<br/>src/lib.rs:95-98",
                "title": "Falta de canonização de charge_id e machine_id contorna DuplicateOrder na mesma estação",
                "description_html": (
                    "<font name='Mono'>Station::new</font> (linha 51) e <font name='Mono'>create_order</font> (linha 72) "
                    "validam <font name='Mono'>.trim().is_empty()</font>, mas armazenam e comparam a string "
                    "<b>bruta sem normalização</b> no <font name='Mono'>HashMap</font> e em "
                    "<font name='Mono'>machine_id != self.machine_id</font>."
                ),
                "exploitability_html": (
                    "Chamadas <font name='Mono'>create_order(\"pix-1\", ...)</font> e "
                    "<font name='Mono'>create_order(\"pix-1 \", ...)</font> são ambas aceitas na mesma "
                    "<font name='Mono'>Station</font> sem retornar <font name='Mono'>Error::DuplicateOrder</font>, criando "
                    "duas entradas distintas para o mesmo ID lógico e quebrando idempotência se integrações tratarem espaços de forma divergente."
                ),
                "condition_html": (
                    "Requer entrada de <font name='Mono'>charge_id</font>/<font name='Mono'>machine_id</font> com espaços "
                    "nas extremidades ou variação de caixa sem higienização prévia."
                ),
                "code": (
                    "71: let charge_id = charge_id.into();\n"
                    "72: if charge_id.trim().is_empty() || amount_cents == 0\n"
                    "       || duration_seconds == 0 {\n"
                    "73:     return Err(Error::InvalidOrder);\n"
                    "75: if self.orders.contains_key(&charge_id) { // sem trim"
                ),
            }
        ],
    },
    {
        "cat_num": 4,
        "cat_title": "Categoria 4 — Chaves Expostas (Hardcode, Defaults, CI e Histórico Git)",
        "applicability": (
            "<b>Aplicabilidade à stack:</b> <i>Totalmente aplicável</i> (6 arquivos, <font name='Mono'>Cargo.toml</font>, "
            "CI e 7 commits do histórico Git; frontend bundle: <i>N/A</i>). <b>Zero segredos hardcoded, tokens, senhas ou "
            "defaults inseguros encontrados</b>; registramos 2 melhorias preventivas de repositório e CI."
        ),
        "findings": [
            {
                "id": "F-04",
                "severity": "Baixa",
                "location_html": ".gitignore:1",
                "title": ".gitignore ignora apenas /target/ e não bloqueia .env, chaves privadas ou certificados TLS",
                "description_html": (
                    "<font name='Mono'>.gitignore</font> contém apenas <font name='Mono'>/target/</font>, sem regras "
                    "defensivas para <font name='Mono'>.env</font>, <font name='Mono'>.env.*</font>, "
                    "<font name='Mono'>*.pem</font>, <font name='Mono'>*.key</font>, <font name='Mono'>*.p12</font> ou "
                    "<font name='Mono'>*.pfx</font>."
                ),
                "exploitability_html": (
                    "Como o plano (<font name='Mono'>README.md:50-56</font>) prevê credenciais Pix e TLS por máquina, "
                    "um <font name='Mono'>git add .</font> futuro pode vazar segredos locais no histórico Git."
                ),
                "condition_html": (
                    "Achado preventivo — nenhum segredo existe no repositório hoje; materializa-se nas próximas entregas."
                ),
                "code": "1: /target/",
            },
            {
                "id": "F-06",
                "severity": "Informativa",
                "location_html": ".github/workflows/<br/>rust.yml:12",
                "title": "Action de CI fixada por tag mutável (@v4) em vez de commit SHA imutável",
                "description_html": (
                    "O workflow referencia <font name='Mono'>actions/checkout@v4</font> por tag móvel. Hoje o risco é "
                    "reduzido por <font name='Mono'>permissions: contents: read</font> (linhas 3-4) e ausência de segredos."
                ),
                "exploitability_html": (
                    "Em comprometimento de tag upstream, código não revisado rodaria no CI, elevando risco de exfiltração "
                    "quando segredos (ex.: assinatura de binário Windows) forem adicionados."
                ),
                "condition_html": (
                    "Requer comprometimento da tag upstream <font name='Mono'>@v4</font>; impacto relevante quando o CI usar segredos."
                ),
                "code": "12: - uses: actions/checkout@v4",
            },
        ],
    },
    {
        "cat_num": 5,
        "cat_title": "Categoria 5 — Inputs sem Tratamento (XSS e Validação de Entradas)",
        "applicability": (
            "<b>Aplicabilidade à stack:</b> <i>XSS não aplicável (N/A)</i> — não existem frontend, DOM "
            "(<font name='Mono'>innerHTML</font>/<font name='Mono'>dangerouslySetInnerHTML</font>/<font name='Mono'>v-html</font>), "
            "Markdown/HTML, URLs <font name='Mono'>href</font>/<font name='Mono'>src</font>, <font name='Mono'>eval</font>, "
            "templates HTML ou envio de e-mails. Auditamos as entradas aceitas pela biblioteca Rust."
        ),
        "findings": [
            {
                "id": "F-05",
                "severity": "Baixa",
                "location_html": "src/lib.rs:32-36<br/>src/lib.rs:42<br/>src/lib.rs:49-55<br/>src/lib.rs:65-84",
                "title": "Identificadores aceitam caracteres de controle e tamanho irrestrito; orders cresce sem limite/TTL",
                "description_html": (
                    "<font name='Mono'>Station::new</font> e <font name='Mono'>create_order</font> não rejeitam "
                    "caracteres de controle (<font name='Mono'>\\r</font>, <font name='Mono'>\\n</font>, "
                    "<font name='Mono'>\\0</font>, sequências ANSI) nem limitam comprimento de IDs. "
                    "<font name='Mono'>Order</font> não possui TTL e <font name='Mono'>self.orders</font> cresce sem teto."
                ),
                "exploitability_html": (
                    "Caracteres de controle em <font name='Mono'>machine_id</font>/<font name='Mono'>charge_id</font> "
                    "viabilizam <i>log injection</i> ao gravar auditorias; crescimento ilimitado de "
                    "<font name='Mono'>self.orders</font> permite exaustão de memória (DoS) sob criação repetida de ordens."
                ),
                "condition_html": (
                    "Explorável quando IDs vierem de entrada externa não filtrada ou quando clientes puderem invocar "
                    "<font name='Mono'>create_order</font> repetidamente em serviço contínuo."
                ),
                "code": (
                    "42: orders: HashMap<String, Order>, // sem limite ou TTL\n"
                    "51: if machine_id.trim().is_empty() {\n"
                    "72: if charge_id.trim().is_empty() || amount_cents == 0 {"
                ),
            }
        ],
    },
]

GITHUB_ISSUES = [
    """--- ISSUE 1 ---
Título: [Segurança] Garantir unicidade global de charge_id entre estações e persistência transacional de idempotência
Labels: security, severity:medium

## Descrição do problema
Na implementação atual (`src/lib.rs`), cada instância de `Station` mantém seu próprio mapa `orders: HashMap<String, Order>` apenas em RAM. O método `create_order` checa duplicidade exclusivamente dentro de `self.orders`.

Por que é explorável:
- Se o futuro servidor gerenciar múltiplas estações (`PC-01`, `PC-02`, ...) confiando apenas na struct `Station`, o mesmo `charge_id` pode ser registrado e confirmado em duas máquinas distintas sem disparar `Error::DuplicateOrder`.
- Em caso de reinicialização do processo, `self.orders` é perdido, permitindo que um `charge_id` já creditado seja aceito novamente (replay de pagamento).

Condição: materializa-se ao integrar `comunicacao-lan` a um servidor multi-estação sem persistência transacional com unicidade global de `charge_id`.

## Evidência
Arquivo: `src/lib.rs:32-46`, `src/lib.rs:65-84`, `src/lib.rs:88-112`

```rust
pub struct Station {
    machine_id: String,
    orders: HashMap<String, Order>,
    ...
}
// create_order (linhas 75-77):
if self.orders.contains_key(&charge_id) {
    return Err(Error::DuplicateOrder);
}
```

## Impacto
Concessão indevida de tempo em múltiplas máquinas com uma única cobrança Pix ou reaplicação do mesmo pagamento após reinício do servidor.

## Sugestão de correção
1. Introduzir repositório transacional no servidor (SQLite/PostgreSQL) com restrição global `UNIQUE(charge_id)` e vínculo `(charge_id, machine_id, amount_cents, status)`.
2. Executar a transição `pending -> credited` e a extensão de prazo na mesma transação atômica.
3. Opcionalmente, criar um agregador `LanRegistry` no domínio que imponha unicidade global entre instâncias `Station` em testes em memória.

## Critérios de aceite
- [ ] Registrar o mesmo `charge_id` em duas estações distintas falha com erro de duplicidade.
- [ ] Reiniciar o serviço mantém cobranças creditadas e rejeita reprocessamento sem somar tempo.
- [ ] Teste automatizado cobrindo confirmação cruzada entre duas máquinas e cenário de reinício.
--- FIM ISSUE 1 ---""",
    """--- ISSUE 2 ---
Título: [Segurança] Restringir confirm_payment e relógio confiável com tipos selados de capacidade
Labels: security, severity:medium

## Descrição do problema
Os métodos críticos `Station::confirm_payment` e `Station::create_order` são `pub fn` recebendo primitivos brutos (`&str`, `u64`), incluindo o timestamp `now: u64`. A regra de que apenas o servidor (após validação autenticada no provedor Pix) pode confirmar pagamentos e fornecer um relógio confiável é mantida somente em comentários (`src/lib.rs:1-3, 38-39, 86-87` e `README.md:30-33`).

Por que é explorável:
- Sem imposição em tempo de compilação pelo sistema de tipos do Rust, qualquer futuro módulo no workspace (handler HTTP, IPC local ou interface) pode invocar `confirm_payment` diretamente sem passar pelo adaptador Pix verificado, ou fornecer `now` manipulado.

Condição: aplicável quando a biblioteca for vinculada por binários futuros (servidor HTTP, serviço Windows e interface).

## Evidência
Arquivo: `src/lib.rs:65-70`, `src/lib.rs:88-112`, `src/lib.rs:153-158`

```rust
pub fn confirm_payment(
    &mut self,
    charge_id: &str,
    machine_id: &str,
    amount_cents: u64,
    now: u64,
) -> Result<Vec<Event>, Error>
```

## Impacto
Risco de regressão silenciosa na fronteira de confiança entre camadas (liberação de máquina sem validação real do provedor Pix ou alteração indevida de prazo).

## Sugestão de correção
1. Criar tipos selados/opacos (`VerifiedPayment` construtível apenas pelo adaptador Pix autenticado e `TrustedInstant` emitido apenas por relógio monotônico/autenticado).
2. Exigir `VerifiedPayment` em `Station::confirm_payment` e tipo de plano do catálogo em `Station::create_order`.
3. Separar em crates distintas no workspace o núcleo do servidor (privilegiado) e os contratos consumidos pela interface, garantindo que a UI não tenha acesso de compilação a `confirm_payment`.

## Critérios de aceite
- [ ] `Station::confirm_payment` aceita somente `VerifiedPayment` não construtível fora do adaptador validado (ou módulo `#[cfg(test)]`).
- [ ] O binário da interface da estação não compila caso tente invocar `confirm_payment`.
- [ ] Existe abstração explícita de relógio confiável separada de inteiros `u64` arbitrários.
--- FIM ISSUE 2 ---""",
    """--- ISSUE 3 ---
Título: [Segurança] Canonizar identificadores, validar caracteres de controle e limitar crescimento de ordens em Station
Labels: security, severity:low

## Descrição do problema
Agrupa dois achados de validação de entrada e resolução por ID em `src/lib.rs` (F-03 e F-05):
1. `Station::new` e `Station::create_order` checam `.trim().is_empty()`, mas armazenam e comparam strings brutas. Assim, `"pix-1"` e `"pix-1 "` coexistem em `self.orders` sem disparar `Error::DuplicateOrder`.
2. `machine_id` e `charge_id` aceitam caracteres de controle (`\\r`, `\\n`, `\\0`, ANSI) e tamanho irrestrito, viabilizando *log injection* em logs futuros.
3. O mapa `self.orders` não possui limite de entradas nem expiração (TTL) de ordens não pagas, permitindo crescimento ilimitado de memória.

Por que é explorável:
- Normalização divergente de espaços entre integrações pode burlar `DuplicateOrder` ou falhar ao localizar ordens.
- Quebras de linha em IDs podem forjar linhas de auditoria, e criação repetida de ordens sem expiração pode causar exaustão de memória (DoS).

Condição: requer que IDs ou `create_order` recebam entradas externas sem saneamento/rate-limit prévio.

## Evidência
Arquivo: `src/lib.rs:32-36`, `src/lib.rs:42`, `src/lib.rs:49-55`, `src/lib.rs:65-84`, `src/lib.rs:95-98`

```rust
pub struct Station {
    machine_id: String,
    orders: HashMap<String, Order>, // sem limite ou TTL
    ...
}
// create_order (linhas 71-78):
let charge_id = charge_id.into();
if charge_id.trim().is_empty() || amount_cents == 0 || duration_seconds == 0 {
    return Err(Error::InvalidOrder);
}
if self.orders.contains_key(&charge_id) {
    return Err(Error::DuplicateOrder);
}
```

## Impacto
Inconsistência na checagem de duplicidade de cobranças, injeção em logs de auditoria e consumo ilimitado de memória em execução contínua.

## Sugestão de correção
1. Criar `validate_identifier(id: &str) -> Result<(), Error>` rejeitando espaços nas extremidades (`id != id.trim()`), caracteres de controle (`is_control()`), caracteres fora de `[A-Za-z0-9_-]` e tamanho acima de 64–128 bytes.
2. Aplicar a validação em `Station::new`, `create_order` e `confirm_payment`.
3. Adicionar `expires_at` às ordens pendentes e limite configurável com limpeza de ordens expiradas não pagas.

## Critérios de aceite
- [ ] `Station::new` e `create_order` rejeitam IDs com espaços laterais, caracteres de controle (`\\n`, `\\r`, `\\0`, ESC) ou comprimento excessivo.
- [ ] `"pix-1"` e `"pix-1 "` não coexistem como ordens distintas na mesma estação.
- [ ] Ordens pendentes possuem expiração/limite máximo testado em `cargo test --all-targets`.
--- FIM ISSUE 3 ---""",
    """--- ISSUE 4 ---
Título: [Segurança] Blindar .gitignore contra segredos locais e fixar GitHub Actions por SHA
Labels: security, severity:low, ci

## Descrição do problema
Agrupa dois achados preventivos de higiene de segredos e cadeia de suprimentos (F-04 e F-06):
1. `.gitignore` contém apenas `/target/`, sem exclusões para arquivos de ambiente (`.env`, `.env.*`), chaves privadas e certificados TLS (`*.pem`, `*.key`, `*.p12`, `*.pfx`) previstos no plano (`README.md:50-56`).
2. `.github/workflows/rust.yml` referencia `actions/checkout@v4` por tag major mutável em vez de commit SHA completo de 40 caracteres.

Por que é explorável:
- Sem regras no `.gitignore`, credenciais Pix ou certificados locais podem ser commitados acidentalmente com `git add .` e expostos no histórico Git.
- Actions por tag mutável expõem o runner de CI a comprometimento upstream, ampliando risco quando segredos ou assinatura de binários forem adicionados ao pipeline.

Condição: achado preventivo — nenhum segredo existe no repositório ou no CI atualmente.

## Evidência
Arquivo: `.gitignore:1` e `.github/workflows/rust.yml:12`

```gitignore
/target/
```

```yaml
- uses: actions/checkout@v4
```

## Impacto
Risco futuro de vazamento acidental de credenciais Pix/TLS no histórico Git e exposição da pipeline de CI a alteração indevida de Action externa.

## Sugestão de correção
1. Expandir `.gitignore` com `.env`, `.env.*`, `!.env.example`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.crt`, `secrets/`, `config.local.*`.
2. Fixar `actions/checkout` em `.github/workflows/rust.yml:12` para o commit SHA completo de 40 caracteres (mantendo `# v4` como comentário) e habilitar Dependabot para atualizações seguras.

## Critérios de aceite
- [ ] `.gitignore` bloqueia `.env`, `.env.*` (exceto `.env.example`) e extensões de chaves/certificados (`*.pem`, `*.key`, `*.p12`, `*.pfx`).
- [ ] `git check-ignore .env cert.pem private.key` confirma que todos são ignorados.
- [ ] `.github/workflows/rust.yml` usa SHA completo nas Actions e o workflow segue verde em Linux e Windows.
--- FIM ISSUE 4 ---""",
]


# ---------------------------------------------------------------------------
# Montagem do PDF
# ---------------------------------------------------------------------------
def build_pdf(output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    styles = build_styles()

    margin = 2.0 * cm
    doc = SimpleDocTemplate(
        str(output_path),
        pagesize=A4,
        leftMargin=margin,
        rightMargin=margin,
        topMargin=1.95 * cm,
        bottomMargin=1.80 * cm,
        title=REPORT_TITLE,
        author="Auditoria de Segurança — comunicacao-lan",
        subject="Revisão de segurança em 5 categorias",
    )
    content_width = A4[0] - 2 * margin
    story: list = []

    # ======================================================================
    # PÁGINA 1 — CAPA + STACK + NOTA METODOLÓGICA (TUDO EM 1 PÁGINA LIMPA)
    # ======================================================================
    story.append(Spacer(1, 0.05 * cm))
    story.append(Paragraph("AUDITORIA ESTÁTICA DE SEGURANÇA DE CÓDIGO", styles["cover_kicker"]))
    story.append(Paragraph(REPORT_TITLE, styles["cover_title"]))
    story.append(
        Paragraph(
            "Revisão linha por linha orientada a cinco categorias críticas de segurança, "
            "adaptada à stack Rust detectada no repositório.",
            styles["cover_sub"],
        )
    )
    # Espaço até sair do banner escuro (5.75 cm a partir do topo - 1.95 cm topMargin = ~3.8 cm úteis no banner)
    story.append(Spacer(1, 0.78 * cm))

    meta_rows = [
        [
            Paragraph("<b>Projeto / Crate</b>", styles["cell_bold"]),
            Paragraph(
                f"<font name='{FONT_MONO}'>{REPO_NAME}</font> (<font name='{FONT_MONO}'>v0.1.0</font>)",
                styles["cell_compact"],
            ),
            Paragraph("<b>Data</b>", styles["cell_bold"]),
            Paragraph(AUDIT_DATE, styles["cell_compact"]),
        ],
        [
            Paragraph("<b>Branch / Commit</b>", styles["cell_bold"]),
            Paragraph(
                f"<font name='{FONT_MONO}'>main</font> · <font name='{FONT_MONO}'>{COMMIT_HASH}</font> (7 commits)",
                styles["cell_compact"],
            ),
            Paragraph("<b>Testes</b>", styles["cell_bold"]),
            Paragraph(
                f"<font name='{FONT_BOLD}' color='{HEX_FORTE}'>8/8 aprovados</font> (<font name='{FONT_MONO}'>cargo test</font>)",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("<b>Escopo Auditado</b>", styles["cell_bold"]),
            Paragraph(
                "100% dos 6 arquivos (<font name='Mono'>src/lib.rs</font>, "
                "<font name='Mono'>examples/session_demo.rs</font>, <font name='Mono'>Cargo.toml</font>, "
                "<font name='Mono'>.github/workflows/rust.yml</font>, <font name='Mono'>.gitignore</font>, "
                "<font name='Mono'>README.md</font>) e histórico Git completo.",
                styles["cell_compact"],
            ),
            Paragraph("<b>Status Geral</b>", styles["cell_bold"]),
            SeverityChip("0 Críticas / 0 Altas", HEX_FORTE, width=3.15 * cm, height=0.48 * cm),
        ],
    ]
    meta_tbl = Table(meta_rows, colWidths=[2.95 * cm, 6.75 * cm, 2.55 * cm, content_width - 12.25 * cm])
    meta_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, -1), C_SLATE_50),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
                ("TOPPADDING", (0, 0), (-1, -1), 4.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 4.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 6),
                ("RIGHTPADDING", (0, 0), (-1, -1), 6),
            ]
        )
    )
    story.append(meta_tbl)
    story.append(Spacer(1, 0.28 * cm))

    story.append(Paragraph("Stack Detectada", styles["h2"]))
    stack_rows = [
        [
            Paragraph("Camada", styles["th"]),
            Paragraph("Detecção Real no Repositório", styles["th"]),
        ],
        [
            Paragraph("Linguagem e Build", styles["cell_bold"]),
            Paragraph(
                "Rust (Edition 2021, <font name='Mono'>Cargo.toml:4</font>), crate de biblioteca "
                "(<font name='Mono'>src/lib.rs</font>) + exemplo CLI (<font name='Mono'>examples/session_demo.rs</font>), "
                "<b>0 dependências externas</b> (<font name='Mono'>Cargo.toml:8</font>) e <font name='Mono'>publish = false</font>.",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("Banco / ORM / HTTP", styles["cell_bold"]),
            Paragraph(
                "<b>Ausentes</b>. Armazenamento exclusivo em memória via <font name='Mono'>HashMap&lt;String, Order&gt;</font> "
                "por instância de <font name='Mono'>Station</font> (<font name='Mono'>src/lib.rs:42</font>); sem servidor HTTP ou rotas.",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("Auth / Frontend / Deploy", styles["cell_bold"]),
            Paragraph(
                "<b>Sem autenticação nem frontend web</b>. Automação restrita ao CI GitHub Actions "
                "(<font name='Mono'>.github/workflows/rust.yml</font>, Linux + Windows); sem Docker/Helm/Terraform.",
                styles["cell_compact"],
            ),
        ],
    ]
    stack_tbl = Table(stack_rows, colWidths=[3.55 * cm, content_width - 3.55 * cm])
    stack_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_SLATE_800),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, C_SLATE_50]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("TOPPADDING", (0, 0), (-1, -1), 4.2),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 4.2),
                ("LEFTPADDING", (0, 0), (-1, -1), 6),
                ("RIGHTPADDING", (0, 0), (-1, -1), 6),
            ]
        )
    )
    story.append(stack_tbl)
    story.append(Spacer(1, 0.25 * cm))

    story.append(Paragraph("Nota Metodológica — Adaptação das 5 Categorias à Stack", styles["h2"]))
    story.append(
        Paragraph(
            "Para evitar falsos positivos ou achados forçados, cada categoria foi mapeada ao equivalente real "
            "desta base Rust, declarando explicitamente o que não se aplica na forma literal:",
            styles["body"],
        )
    )

    meth_rows = [
        [
            Paragraph("Categoria", styles["th"]),
            Paragraph("Status na Stack", styles["th"]),
            Paragraph("Mapeamento Aplicado na Auditoria", styles["th"]),
        ],
        [
            Paragraph("1. Banco sem Tranca", styles["cell_bold"]),
            Paragraph("Parcial (RAM)", styles["cell_compact"]),
            Paragraph(
                "Sem SQL/RLS; auditado o isolamento de <font name='Mono'>orders</font> por "
                "<font name='Mono'>Station</font> e unicidade/idempotência de <font name='Mono'>charge_id</font> entre máquinas e reinícios.",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("2. Permissão no Navegador", styles["cell_bold"]),
            Paragraph("N/A web / Adaptada", styles["cell_compact"]),
            Paragraph(
                "Sem navegador/RBAC/HTTP; auditada a fronteira de confiança dos métodos públicos privilegiados "
                "(<font name='Mono'>confirm_payment</font>, <font name='Mono'>create_order</font>) e do relógio <font name='Mono'>now</font>.",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("3. IDOR", styles["cell_bold"]),
            Paragraph("N/A HTTP / Adaptada", styles["cell_compact"]),
            Paragraph(
                "Sem rotas HTTP; percorridos todos os 5 métodos públicos de <font name='Mono'>Station</font> "
                "quanto à validação de posse (<font name='Mono'>machine_id</font>) e resolução de objetos por ID.",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("4. Chaves Expostas", styles["cell_bold"]),
            Paragraph("Aplicável (total)", styles["cell_compact"]),
            Paragraph(
                "Inspecionados todos os 6 arquivos, <font name='Mono'>Cargo.toml</font>, <font name='Mono'>.gitignore</font>, "
                "CI GitHub Actions e os 7 commits do histórico Git. Frontend bundle: N/A.",
                styles["cell_compact"],
            ),
        ],
        [
            Paragraph("5. Inputs / XSS", styles["cell_bold"]),
            Paragraph("N/A XSS / Adaptada", styles["cell_compact"]),
            Paragraph(
                "Sem DOM, templates HTML ou e-mails (XSS N/A); auditada a validação de strings "
                "(<font name='Mono'>machine_id</font>, <font name='Mono'>charge_id</font>) e limites de <font name='Mono'>Station::orders</font>.",
                styles["cell_compact"],
            ),
        ],
    ]
    meth_tbl = Table(meth_rows, colWidths=[3.35 * cm, 2.85 * cm, content_width - 6.20 * cm])
    meth_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_SLATE_800),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, C_SLATE_50]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("TOPPADDING", (0, 0), (-1, -1), 4.2),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 4.2),
                ("LEFTPADDING", (0, 0), (-1, -1), 6),
                ("RIGHTPADDING", (0, 0), (-1, -1), 6),
            ]
        )
    )
    story.append(meth_tbl)
    story.append(PageBreak())

    # ======================================================================
    # PÁGINA 2 — RESUMO EXECUTIVO + GRÁFICOS
    # ======================================================================
    story.append(SectionHeader("1.", "Resumo Executivo", content_width))
    story.append(Spacer(1, 0.25 * cm))
    story.append(
        Paragraph(
            "A auditoria cobriu <b>100% do código-fonte, configurações, workflow de CI, documentação e histórico Git</b> "
            "do repositório <font name='Mono'>BozZf/comunicacao-lan</font>. Por se tratar da primeira base do núcleo "
            "de regras de sessão em Rust (sem servidor HTTP, banco de dados, integração Pix real ou frontend web), "
            "<b>não foram encontradas vulnerabilidades de severidade Crítica ou Alta</b>, tampouco segredos vazados no "
            "código ou no histórico de commits.",
            styles["body"],
        )
    )
    story.append(
        Paragraph(
            "Foram verificados <b>6 achados reais</b> (2 Médios, 3 Baixos e 1 Informativo) — todos acompanhados de suas "
            "condições exatas de explorabilidade — e <b>6 pontos fortes</b> comprovando controles sólidos no núcleo de domínio "
            "e na esteira de CI. Os dois achados Médios representam riscos arquiteturais que se tornarão exploráveis assim "
            "que a biblioteca for integrada ao futuro servidor multi-estação caso não sejam endereçados antes do piloto.",
            styles["body"],
        )
    )
    story.append(Spacer(1, 0.22 * cm))

    kpi_specs = [
        ("0", "CRÍTICA", HEX_CRITICA, "#FEF2F2"),
        ("0", "ALTA", HEX_ALTA, "#FFF7ED"),
        ("2", "MÉDIA", HEX_MEDIA, "#FFFBEB"),
        ("3", "BAIXA", HEX_BAIXA, "#EFF6FF"),
        ("1", "INFORMATIVA", HEX_INFO, "#F8FAFC"),
        ("6", "PONTOS FORTES", HEX_FORTE, "#ECFDF5"),
    ]
    kpi_cells = []
    for val, lbl, fg_hex, bg_hex in kpi_specs:
        p_val = Paragraph(f"<font color='{fg_hex}'>{val}</font>", styles["kpi_val"])
        p_lbl = Paragraph(lbl, styles["kpi_lbl"])
        sub = Table([[p_val], [p_lbl]], colWidths=[(content_width / 6.0) - 0.14 * cm])
        sub.setStyle(
            TableStyle(
                [
                    ("BACKGROUND", (0, 0), (-1, -1), colors.HexColor(bg_hex)),
                    ("LINEABOVE", (0, 0), (-1, 0), 2.8, colors.HexColor(fg_hex)),
                    ("BOX", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                    ("TOPPADDING", (0, 0), (-1, 0), 7),
                    ("BOTTOMPADDING", (0, 0), (-1, 0), 2),
                    ("TOPPADDING", (0, 1), (-1, 1), 1),
                    ("BOTTOMPADDING", (0, 1), (-1, 1), 6),
                ]
            )
        )
        kpi_cells.append(sub)

    kpi_row = Table([kpi_cells], colWidths=[content_width / 6.0] * 6)
    kpi_row.setStyle(
        TableStyle(
            [
                ("LEFTPADDING", (0, 0), (-1, -1), 2),
                ("RIGHTPADDING", (0, 0), (-1, -1), 2),
                ("TOPPADDING", (0, 0), (-1, -1), 0),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 0),
            ]
        )
    )
    story.append(kpi_row)
    story.append(Spacer(1, 0.38 * cm))

    donut_buf = build_donut_chart()
    bar_buf = build_category_bar_chart()

    chart_w_left = content_width * 0.485
    chart_w_right = content_width * 0.505
    donut_img = Image(donut_buf, width=chart_w_left - 0.25 * cm, height=(chart_w_left - 0.25 * cm) * (3.15 / 4.4))
    bar_img = Image(bar_buf, width=chart_w_right - 0.25 * cm, height=(chart_w_right - 0.25 * cm) * (3.15 / 4.6))

    charts_tbl = Table([[donut_img, bar_img]], colWidths=[chart_w_left, chart_w_right])
    charts_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, -1), colors.white),
                ("BOX", (0, 0), (0, 0), 0.5, C_SLATE_200),
                ("BOX", (1, 0), (1, 0), 0.5, C_SLATE_200),
                ("ALIGN", (0, 0), (-1, -1), "CENTER"),
                ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
                ("TOPPADDING", (0, 0), (-1, -1), 6),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 6),
                ("LEFTPADDING", (0, 0), (-1, -1), 3),
                ("RIGHTPADDING", (0, 0), (-1, -1), 3),
            ]
        )
    )
    story.append(charts_tbl)
    story.append(Spacer(1, 0.35 * cm))

    story.append(Paragraph("Quadro-Resumo por Categoria Auditada", styles["h2"]))
    summary_rows = [
        [
            Paragraph("Categoria", styles["th"]),
            Paragraph("Aplicabilidade", styles["th"]),
            Paragraph("Achados", styles["th"]),
            Paragraph("Maior Sev.", styles["th"]),
            Paragraph("Síntese", styles["th"]),
        ],
        [
            Paragraph("1. Banco sem Tranca", styles["cell_bold"]),
            Paragraph("Parcial (RAM)", styles["cell"]),
            Paragraph("1 (F-01)", styles["cell"]),
            SeverityChip("Média", HEX_MEDIA, width=1.80 * cm, height=0.45 * cm),
            Paragraph("Isolamento por instância OK; falta unicidade global de charge_id e persistência.", styles["cell"]),
        ],
        [
            Paragraph("2. Permissão no Navegador", styles["cell_bold"]),
            Paragraph("N/A web / Rust", styles["cell"]),
            Paragraph("1 (F-02)", styles["cell"]),
            SeverityChip("Média", HEX_MEDIA, width=1.80 * cm, height=0.45 * cm),
            Paragraph("Sem frontend/HTTP; confirm_payment aceita primitivos sem tipo selado.", styles["cell"]),
        ],
        [
            Paragraph("3. IDOR", styles["cell_bold"]),
            Paragraph("N/A HTTP / Rust", styles["cell"]),
            Paragraph("1 (F-03)", styles["cell"]),
            SeverityChip("Baixa", HEX_BAIXA, width=1.80 * cm, height=0.45 * cm),
            Paragraph("5/5 métodos validam posse de máquina; falta canonização de IDs (trim).", styles["cell"]),
        ],
        [
            Paragraph("4. Chaves Expostas", styles["cell_bold"]),
            Paragraph("Total (repo+git)", styles["cell"]),
            Paragraph("2 (F-04, F-06)", styles["cell"]),
            SeverityChip("Baixa", HEX_BAIXA, width=1.80 * cm, height=0.45 * cm),
            Paragraph("Zero segredos vazados; .gitignore minimalista e Action @v4 sem SHA.", styles["cell"]),
        ],
        [
            Paragraph("5. Inputs / XSS", styles["cell_bold"]),
            Paragraph("N/A XSS / Rust", styles["cell"]),
            Paragraph("1 (F-05)", styles["cell"]),
            SeverityChip("Baixa", HEX_BAIXA, width=1.80 * cm, height=0.45 * cm),
            Paragraph("XSS N/A; IDs aceitam chars de controle e self.orders cresce sem TTL.", styles["cell"]),
        ],
    ]
    sum_tbl = Table(
        summary_rows,
        colWidths=[3.15 * cm, 2.65 * cm, 1.95 * cm, 2.05 * cm, content_width - 9.80 * cm],
    )
    sum_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_SLATE_900),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, C_SLATE_50]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
                ("TOPPADDING", (0, 0), (-1, -1), 5.2),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 5.2),
                ("LEFTPADDING", (0, 0), (-1, -1), 5.5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 5.5),
            ]
        )
    )
    story.append(sum_tbl)
    story.append(PageBreak())

    # ======================================================================
    # PÁGINA 3 — PONTOS FORTES & PONTOS FRACOS
    # ======================================================================
    story.append(SectionHeader("2.", "Pontos Fortes e Pontos Fracos", content_width))
    story.append(Spacer(1, 0.25 * cm))

    story.append(
        Paragraph(
            f"<font color='{HEX_FORTE}'>●</font> Pontos Fortes — Controles Verificados como Corretos",
            styles["h2"],
        )
    )
    story.append(
        Paragraph(
            "Os controles abaixo foram verificados diretamente no código-fonte, nos testes automatizados e no histórico "
            "Git, demonstrando boa disciplina de engenharia de domínio e higiene inicial de segurança:",
            styles["body"],
        )
    )

    strengths_rows = [
        [
            Paragraph("Status", styles["th"]),
            Paragraph("Evidência", styles["th"]),
            Paragraph("Controle Verificado", styles["th"]),
        ],
        [
            SeverityChip("Ponto Forte", HEX_FORTE, width=2.05 * cm, height=0.46 * cm),
            Paragraph("src/lib.rs:31-46", styles["cell_mono"]),
            Paragraph(
                "<b>Encapsulamento estrito de invariantes:</b> todos os campos de <font name='Mono'>Station</font> "
                "(<font name='Mono'>machine_id</font>, <font name='Mono'>orders</font>, <font name='Mono'>deadline</font>, "
                "<font name='Mono'>last_now</font>, <font name='Mono'>warned</font>) e a struct <font name='Mono'>Order</font> "
                "são privados, impedindo adulteração direta de prazo ou status de pagamento por código externo.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("Ponto Forte", HEX_FORTE, width=2.05 * cm, height=0.46 * cm),
            Paragraph("src/lib.rs:95-111", styles["cell_mono"]),
            Paragraph(
                "<b>Validação completa antes de creditar (fail-closed):</b> <font name='Mono'>confirm_payment</font> "
                "verifica existência da ordem, correspondência de <font name='Mono'>machine_id</font> e "
                "<font name='Mono'>amount_cents</font>, monotonicidade do relógio, idempotência na instância e overflow "
                "via <font name='Mono'>checked_add</font> (linha 105) <i>antes</i> de mutar qualquer estado.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("Ponto Forte", HEX_FORTE, width=2.05 * cm, height=0.46 * cm),
            Paragraph("src/lib.rs:114-158", styles["cell_mono"]),
            Paragraph(
                "<b>Proteção contra relógio regressivo e expiração offline:</b> <font name='Mono'>check_time</font> "
                "rejeita <font name='Mono'>now &lt; last_now</font>; <font name='Mono'>access</font> evita underflow em "
                "<font name='Mono'>deadline - now</font>; <font name='Mono'>tick</font> emite "
                "<font name='Mono'>Event::Locked</font> exatamente uma vez ao expirar, mesmo sem rede.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("Ponto Forte", HEX_FORTE, width=2.05 * cm, height=0.46 * cm),
            Paragraph("src/lib.rs:161-246", styles["cell_mono"]),
            Paragraph(
                "<b>Cobertura de testes de segurança de domínio:</b> 8 testes unitários (todos passando em "
                "<font name='Mono'>cargo test --all-targets</font>) cobrem idempotência, troca de máquina/valor, "
                "ordem inexistente, rollback de relógio, overflow de <font name='Mono'>u64::MAX</font> e duplicidade.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("Ponto Forte", HEX_FORTE, width=2.05 * cm, height=0.46 * cm),
            Paragraph("Cargo.toml:5,8<br/>git log (7 commits)", styles["cell_mono"]),
            Paragraph(
                "<b>Zero segredos vazados e superfície mínima de dependências:</b> nenhum token, senha, chave ou fallback "
                "<font name='Mono'>${VAR:-default}</font> no código ou histórico Git; zero dependências externas "
                "(<font name='Mono'>[dependencies]</font> vazio) e <font name='Mono'>publish = false</font>.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("Ponto Forte", HEX_FORTE, width=2.05 * cm, height=0.46 * cm),
            Paragraph(".github/workflows/<br/>rust.yml:3-4<br/>README.md:30-33,<br/>48-69", styles["cell_mono"]),
            Paragraph(
                "<b>CI com privilégio mínimo e documentação honesta de fronteiras:</b> workflow define "
                "<font name='Mono'>permissions: contents: read</font> sem expor segredos; "
                "<font name='Mono'>README.md</font> documenta claramente que a crate isolada não constitui fronteira "
                "de segurança e define requisitos contra manipulação de relógio, UAC e herança de sessão no Windows.",
                styles["cell"],
            ),
        ],
    ]
    str_tbl = Table(strengths_rows, colWidths=[2.35 * cm, 3.55 * cm, content_width - 5.90 * cm], repeatRows=1)
    str_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_FORTE),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, colors.HexColor("#F0FDF4")]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("TOPPADDING", (0, 0), (-1, -1), 5.2),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 5.2),
                ("LEFTPADDING", (0, 0), (-1, -1), 5.5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 5.5),
            ]
        )
    )
    story.append(str_tbl)
    story.append(Spacer(1, 0.35 * cm))

    story.append(
        Paragraph(
            f"<font color='{HEX_MEDIA}'>●</font> Pontos Fracos — Riscos Centrais Identificados",
            styles["h2"],
        )
    )
    story.append(
        make_callout(
            "Síntese dos Riscos Centrais antes da Próxima Fase (Servidor + Piloto)",
            "1. <b>Isolamento apenas por instância em RAM (F-01 — Média):</b> <font name='Mono'>Station</font> não impede "
            "reuso do mesmo <font name='Mono'>charge_id</font> entre máquinas distintas nem preserva idempotência após reinício.<br/>"
            "2. <b>Fronteira de autorização apenas documental (F-02 — Média):</b> <font name='Mono'>confirm_payment</font> e "
            "<font name='Mono'>now</font> aceitam primitivos públicos sem tipos selados de capacidade, exigindo disciplina manual "
            "de todos os futuros chamadores.<br/>"
            "3. <b>Higiene de identificadores, memória e repositório (F-03, F-04, F-05, F-06 — Baixa/Info):</b> "
            "falta canonização de IDs (espaços laterais burlam <font name='Mono'>DuplicateOrder</font>), caracteres de controle "
            "não são filtrados, <font name='Mono'>self.orders</font> não tem TTL/limite, <font name='Mono'>.gitignore</font> "
            "não bloqueia <font name='Mono'>.env</font>/chaves e o CI usa tag mutável <font name='Mono'>@v4</font>.",
            HEX_MEDIA,
            "#FFFBEB",
            styles,
            content_width,
        )
    )
    story.append(PageBreak())

    # ======================================================================
    # PÁGINAS 4–6 — ACHADOS DETALHADOS POR CATEGORIA
    # ======================================================================
    story.append(SectionHeader("3.", "Achados Detalhados por Categoria", content_width))
    story.append(Spacer(1, 0.2 * cm))

    for cat in FINDINGS_BY_CATEGORY:
        block = [
            Paragraph(cat["cat_title"], styles["h2"]),
            Paragraph(cat["applicability"], styles["body"]),
            make_findings_table(cat["findings"], styles, content_width),
            Spacer(1, 0.26 * cm),
        ]
        story.append(KeepTogether(block))

    story.append(Spacer(1, 0.25 * cm))

    # ======================================================================
    # SEÇÃO 4 — COBERTURA ARQUIVO A ARQUIVO (flui logo abaixo da Categoria 5)
    # ======================================================================
    cov_rows = [
        [
            Paragraph("Arquivo", styles["th"]),
            Paragraph("Linhas", styles["th"]),
            Paragraph("Achados", styles["th"]),
            Paragraph("Resultado da Inspeção", styles["th"]),
        ],
        [
            Paragraph("src/lib.rs", styles["cell_mono"]),
            Paragraph("1–247", styles["cell"]),
            Paragraph("F-01, F-02,<br/>F-03, F-05", styles["cell_bold"]),
            Paragraph(
                "5 métodos públicos e 8 testes auditados. Posse de máquina, valor, overflow e monotonicidade corretos; "
                "apontados escopo em RAM, falta de tipos selados, canonização e limite/TTL de orders.",
                styles["cell"],
            ),
        ],
        [
            Paragraph("examples/<br/>session_demo.rs", styles["cell_mono"]),
            Paragraph("1–19", styles["cell"]),
            Paragraph("Nenhum", styles["cell"]),
            Paragraph(
                "Exemplo CLI didático com dados fictícios (<font name='Mono'>PC-01</font>, "
                "<font name='Mono'>cobranca-demo</font>); sinaliza simulação na linha 4. Sem segredos ou exposição de rede.",
                styles["cell"],
            ),
        ],
        [
            Paragraph("Cargo.toml", styles["cell_mono"]),
            Paragraph("1–9", styles["cell"]),
            Paragraph("Nenhum", styles["cell"]),
            Paragraph(
                "Crate privada (<font name='Mono'>publish = false</font>, linha 5), zero dependências externas (linha 8), sem credenciais.",
                styles["cell"],
            ),
        ],
        [
            Paragraph(".gitignore", styles["cell_mono"]),
            Paragraph("1–2", styles["cell"]),
            Paragraph("F-04", styles["cell_bold"]),
            Paragraph(
                "Contém apenas <font name='Mono'>/target/</font>. Recomendado adicionar padrões preventivos para "
                "<font name='Mono'>.env</font> e chaves/certificados TLS.",
                styles["cell"],
            ),
        ],
        [
            Paragraph(".github/workflows/<br/>rust.yml", styles["cell_mono"]),
            Paragraph("1–17", styles["cell"]),
            Paragraph("F-06", styles["cell_bold"]),
            Paragraph(
                "Privilégio mínimo (<font name='Mono'>contents: read</font>, linhas 3-4) e sem segredos. "
                "Recomendado fixar <font name='Mono'>actions/checkout</font> (linha 12) por SHA.",
                styles["cell"],
            ),
        ],
        [
            Paragraph("README.md + git log", styles["cell_mono"]),
            Paragraph("1–117<br/>(7 commits)", styles["cell"]),
            Paragraph("Nenhum", styles["cell"]),
            Paragraph(
                "Documentação transparente dos limites atuais e requisitos futuros. Varredura dos 7 commits confirmou "
                "ausência total de segredos ou chaves.",
                styles["cell"],
            ),
        ],
    ]
    cov_tbl = Table(cov_rows, colWidths=[3.55 * cm, 1.80 * cm, 2.05 * cm, content_width - 7.40 * cm], repeatRows=1)
    cov_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_SLATE_800),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, C_SLATE_50]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("TOPPADDING", (0, 0), (-1, -1), 4.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 4.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 5.5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 5.5),
            ]
        )
    )
    story.append(
        KeepTogether(
            [
                SectionHeader("4.", "Cobertura Arquivo por Arquivo (Linha por Linha)", content_width),
                Spacer(1, 0.16 * cm),
                Paragraph(
                    "A tabela abaixo comprova a inspeção integral de todos os artefatos presentes no repositório na revisão "
                    f"<font name='Mono'>{COMMIT_HASH}</font>:",
                    styles["body"],
                ),
                cov_tbl,
            ]
        )
    )
    story.append(Spacer(1, 0.45 * cm))

    # Seção 5 agrupada com KeepTogether para nunca deixar título órfão
    rec_rows = [
        [
            Paragraph("Prioridade", styles["th"]),
            Paragraph("Achados", styles["th"]),
            Paragraph("Ação Recomendada e Resultado Esperado", styles["th"]),
        ],
        [
            SeverityChip("P1 — Piloto", HEX_MEDIA, width=2.85 * cm, height=0.48 * cm),
            Paragraph("F-01, F-02", styles["cell_bold"]),
            Paragraph(
                "<b>Persistência transacional + fronteira tipada:</b> introduzir repositório com "
                "<font name='Mono'>UNIQUE(charge_id)</font> global e transação atômica entre cobrança e crédito; "
                "refatorar <font name='Mono'>confirm_payment</font> para aceitar somente um tipo selado "
                "<font name='Mono'>VerifiedPayment</font> emitido pelo adaptador Pix e abstração de relógio confiável.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("P2 — Imediato", HEX_BAIXA, width=2.85 * cm, height=0.48 * cm),
            Paragraph("F-03, F-05", styles["cell_bold"]),
            Paragraph(
                "<b>Validação estrita de IDs e limite de memória:</b> rejeitar espaços nas extremidades, caracteres "
                "de controle e strings acima de 64–128 bytes em <font name='Mono'>machine_id</font> e "
                "<font name='Mono'>charge_id</font>; adicionar TTL/expiração e teto de ordens pendentes em "
                "<font name='Mono'>Station::orders</font> com testes unitários dedicados.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("P3 — Repo / CI", HEX_INFO, width=2.85 * cm, height=0.48 * cm),
            Paragraph("F-04, F-06", styles["cell_bold"]),
            Paragraph(
                "<b>Prevenção de vazamento e supply-chain CI:</b> adicionar <font name='Mono'>.env*</font> e "
                "extensões de chaves/certificados (<font name='Mono'>*.pem</font>, <font name='Mono'>*.key</font>, "
                "<font name='Mono'>*.p12</font>, <font name='Mono'>*.pfx</font>) ao <font name='Mono'>.gitignore</font>; "
                "fixar <font name='Mono'>actions/checkout</font> por SHA completo de 40 caracteres no workflow.",
                styles["cell"],
            ),
        ],
        [
            SeverityChip("P4 — Roadmap", HEX_FORTE, width=2.85 * cm, height=0.48 * cm),
            Paragraph("Futuro", styles["cell_bold"]),
            Paragraph(
                "<b>Controles para próximas entregas (HTTP/Painel/Pix):</b> implementar RBAC no servidor em toda "
                "rota administrativa, validação obrigatória de segredos no startup (rejeitando defaults), filtro por "
                "máquina/tenant em todas as queries, verificação de assinatura de webhook Pix e sanitização/CSP no painel web.",
                styles["cell"],
            ),
        ],
    ]
    rec_tbl = Table(rec_rows, colWidths=[3.15 * cm, 1.95 * cm, content_width - 5.10 * cm], repeatRows=1)
    rec_tbl.setStyle(
        TableStyle(
            [
                ("BACKGROUND", (0, 0), (-1, 0), C_SLATE_900),
                ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, C_SLATE_50]),
                ("GRID", (0, 0), (-1, -1), 0.45, C_SLATE_200),
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("TOPPADDING", (0, 0), (-1, -1), 5.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 5.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 5.5),
                ("RIGHTPADDING", (0, 0), (-1, -1), 5.5),
            ]
        )
    )
    story.append(
        KeepTogether(
            [
                SectionHeader("5.", "Recomendações Priorizadas", content_width),
                Spacer(1, 0.2 * cm),
                rec_tbl,
            ]
        )
    )
    story.append(PageBreak())

    # ======================================================================
    # PÁGINAS FINAIS — ISSUES PARA O GITHUB (PRONTAS PARA COPIAR E COLAR)
    # ======================================================================
    issues_intro = [
        SectionHeader("6.", "Issues para o GitHub", content_width),
        Spacer(1, 0.18 * cm),
        Paragraph(
            "Os blocos abaixo contêm o texto <b>completo em Markdown</b>, pronto para copiar e colar na abertura de "
            "issues no GitHub. Os 6 achados acionáveis foram agrupados logicamente em <b>4 issues</b> para evitar "
            "spam: Issue 1 (F-01), Issue 2 (F-02), Issue 3 (F-03 + F-05) e Issue 4 (F-04 + F-06).",
            styles["body"],
        ),
        Spacer(1, 0.12 * cm),
    ]

    # Issue 1 acompanha o cabeçalho da Seção 6 na mesma página (sem página vazia!)
    story.append(
        KeepTogether(
            issues_intro
            + [
                Paragraph(f"Issue 1 de {len(GITHUB_ISSUES)}", styles["h3"]),
                make_issue_block(GITHUB_ISSUES[0], styles, content_width),
                Spacer(1, 0.28 * cm),
            ]
        )
    )

    for idx, issue_text in enumerate(GITHUB_ISSUES[1:], start=2):
        block = [
            Paragraph(f"Issue {idx} de {len(GITHUB_ISSUES)}", styles["h3"]),
            make_issue_block(issue_text, styles, content_width),
            Spacer(1, 0.28 * cm),
        ]
        story.append(KeepTogether(block))

    doc.build(story, onFirstPage=draw_cover_background, canvasmaker=AuditCanvas)


def main() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    output_pdf = repo_root / "docs" / "security-audit" / "relatorio-auditoria-seguranca.pdf"
    build_pdf(output_pdf)
    print(f"PDF gerado com sucesso: {output_pdf}")


if __name__ == "__main__":
    main()
