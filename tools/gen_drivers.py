#!/usr/bin/env python3
"""gen_drivers.py — gera os drivers .drv de cada linguagem.

Cada driver de linguagem = SourceCode (silabas PT + sintaxe) + keywords reservadas
marcadas com '=' (Jeito B). O SourceCode_PTBR.drv e a base FIXA — o gerador so
reabre, copia e anexa as keywords da linguagem.

Os drivers BASE (PTBR, SourceCode) sao tratados separadamente: preservam o vocab
atual e apenas regravam o cabecalho expandido.

CABECALHO DOS .drv (4 linhas '#'):
    # RAGnaRock driver: <Lang>
    # descricao: <texto curto>
    # extensoes: .ext1 .ext2 ...
    # base: SourceCode (N silabas)  |  keywords: K
    <silabas/keywords>

Adicionar uma linguagem nova = uma entrada em LANG_INFO. Reexecuta e o driver
aparece em drivers/.

Uso:
    python3 tools/gen_drivers.py                # gera todos os drivers
    python3 tools/gen_drivers.py --only Delphi  # gera so um
    python3 tools/gen_drivers.py --list         # lista linguagens conhecidas
"""
import os, sys, argparse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DRIVERS = os.path.join(ROOT, "drivers")
BASE = os.path.join(DRIVERS, "tokens_SourceCode_PTBR.drv")


# BASE drivers: nao derivam de outro driver — preservam o vocab atual do .drv,
# so reescrevem o cabecalho expandido. Suas silabas sao a "matriz fixa" do projeto.
BASE_DRIVERS = {
    "PTBR": {
        "description": "silabario base do portugues brasileiro (texto literario, prosa)",
        "extensions": [".txt", ".doc", ".docx", ".md", ".markdown", ".rtf", ".odt", ".text", ".log", ".epub"],
    },
    "SourceCode": {
        "description": "base estendida (PT + silabas de codigo, ~95.6% cobertura); fallback generico p/ ext desconhecida",
        "extensions": [],
    },
}


# Drivers DERIVADOS: SourceCode + keywords reservadas da linguagem.
# nome do driver -> {description, extensions[], keywords (string com palavras separadas por espaco)}
LANG_INFO = {
    # --- linguagens originais ---
    "C": {
        "description": "codigo fonte C (ANSI/C99/C11/C17)",
        "extensions": [".c", ".h"],
        "keywords": "auto break case char const continue default do double else enum extern float for goto if inline int long register restrict return short signed sizeof static struct switch typedef union unsigned void volatile while null",
    },
    "Cpp": {
        "description": "codigo fonte C++ (98/11/14/17/20)",
        "extensions": [".cpp", ".cxx", ".cc", ".hpp", ".hxx", ".hh", ".h++", ".inl"],
        "keywords": "auto bool break case catch char class const constexpr continue default delete do double dynamic_cast else enum explicit extern false float for friend goto if inline int long namespace new nullptr operator private protected public register return short signed sizeof static struct switch template this throw true try typedef typename union unsigned using virtual void volatile while std string vector",
    },
    "CSharp": {
        "description": "codigo fonte C# (.NET Framework/.NET Core/.NET 5+)",
        "extensions": [".cs", ".csx"],
        "keywords": "abstract as base bool break byte case catch char checked class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach goto if implicit in int interface internal is lock long namespace new null object operator out override params private protected public readonly ref return sbyte sealed short sizeof static string struct switch this throw true try typeof uint ulong ushort using virtual void volatile while async await var get set record nameof console task list",
    },
    "Cobol": {
        "description": "codigo fonte COBOL (mainframe legacy)",
        "extensions": [".cob", ".cbl", ".cpy", ".cobol"],
        "keywords": "identification division program-id environment configuration data working-storage linkage procedure section pic picture value move to from add subtract multiply divide compute if else end-if perform until varying display accept stop run call using open close read write rewrite goback evaluate when occurs redefines copy input output file fd",
    },
    "Go": {
        "description": "codigo fonte Go (golang)",
        "extensions": [".go"],
        "keywords": "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var nil true false iota make new len cap append string error",
    },
    "Java": {
        "description": "codigo fonte Java (JDK 8+, incluindo records/sealed)",
        "extensions": [".java", ".jav"],
        "keywords": "abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for goto if implements import instanceof int interface long native new package private protected public return short static strictfp super switch synchronized this throw throws transient try void volatile while true false null var record sealed yield string system",
    },
    "JavaScript": {
        "description": "codigo fonte JavaScript (ES5/ES6+/Node.js); inclui JSX",
        "extensions": [".js", ".mjs", ".cjs", ".jsx"],
        "keywords": "async await break case catch class const continue debugger default delete do else export extends false finally for function get if import in instanceof let new null of return set static super switch this throw true try typeof undefined var void while with yield",
    },
    "Kotlin": {
        "description": "codigo fonte Kotlin (JVM/Native/Multiplatform)",
        "extensions": [".kt", ".kts"],
        "keywords": "abstract as break by catch class companion const continue crossinline data do else enum external false final finally for fun if import in infix init inline inner interface internal is lateinit lazy null object open operator out override package private protected public reified return sealed super suspend this throw true try typealias val var vararg when where while",
    },
    "Pascal": {
        "description": "codigo fonte Pascal puro (ISO/Free Pascal); Delphi cobre Object Pascal",
        "extensions": [".pp"],
        "keywords": "and array begin case const div do downto else end file for function goto if implementation in interface label mod nil not of or packed procedure program record repeat set then to type unit until uses var while with class constructor destructor inherited private protected public published property try except finally raise self string boolean integer",
    },
    "Perl": {
        "description": "codigo fonte Perl 5 (scripts e modulos)",
        "extensions": [".pl", ".pm", ".t", ".pod", ".perl"],
        "keywords": "my our local sub use require package if elsif else unless while until for foreach do return last next redo and or not eq ne lt gt le ge cmp print printf say chomp split join grep map sort keys values exists delete defined ref bless wantarray",
    },
    "PHP": {
        "description": "codigo fonte PHP (5.x/7.x/8.x), inclui templates phtml",
        "extensions": [".php", ".phtml", ".php3", ".php4", ".php5", ".php7", ".phps"],
        "keywords": "abstract and array as break callable case catch class clone const continue declare default do echo else elseif empty endif enum extends final finally fn for foreach function global goto if implements include instanceof interface isset list match namespace new or print private protected public readonly require return static switch throw trait try unset use var while xor yield true false null self this",
    },
    "Python": {
        "description": "codigo fonte Python (2.7/3.x, type hints, async/await, match)",
        "extensions": [".py", ".pyw", ".pyi", ".pyx"],
        "keywords": "false none true and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case self print len range int str list dict set tuple bool open",
    },
    "Ruby": {
        "description": "codigo fonte Ruby (incluindo ERB e gemspecs)",
        "extensions": [".rb", ".erb", ".rake", ".gemspec", ".ru"],
        "keywords": "alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield require puts attr_accessor",
    },
    "Rust": {
        "description": "codigo fonte Rust (edicao 2018/2021/2024)",
        "extensions": [".rs"],
        "keywords": "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self super static struct trait true type unsafe use where while some none ok err vec string option result box",
    },
    "Shell": {
        "description": "scripts shell POSIX/Bash/Zsh/Ksh/Csh",
        "extensions": [".sh", ".bash", ".zsh", ".ksh", ".csh", ".bashrc", ".zshrc", ".profile"],
        "keywords": "if then else elif fi case esac for while until do done function in select return break continue local export readonly declare echo printf read exit source alias unset trap shift test cd",
    },
    "SQL": {
        "description": "scripts SQL (DDL/DML), ANSI + dialetos comuns",
        "extensions": [".sql", ".ddl", ".dml"],
        "keywords": "select from where insert update delete create alter drop truncate table view index join inner left right outer full on group by order asc desc having union all distinct as into values set and or not null is in like between exists case when then else end count sum avg min max primary key foreign references default constraint unique check begin commit rollback transaction with limit offset",
    },
    "Swift": {
        "description": "codigo fonte Swift (Apple, iOS/macOS/server-side)",
        "extensions": [".swift"],
        "keywords": "associatedtype class deinit enum extension fileprivate func import init inout internal let open operator private protocol public rethrows static struct subscript typealias var break case continue default defer do else fallthrough for guard if in repeat return switch where while as catch false is nil super self throw throws true try weak unowned lazy mutating override required convenience final",
    },
    "TypeScript": {
        "description": "codigo fonte TypeScript (.ts e .tsx React)",
        "extensions": [".ts", ".tsx", ".mts", ".cts"],
        "keywords": "abstract any as asserts async await boolean break case catch class const continue debugger declare default delete do else enum export extends false finally for from function get if implements import in infer instanceof interface is keyof let module namespace never new null number object of private protected public readonly return set static string super switch this throw true try type typeof undefined unknown var void while yield",
    },

    # --- novas (lote 2026-05-21) ---
    "Delphi": {
        "description": "codigo fonte Delphi / Object Pascal (Embarcadero + Lazarus); cobre .pas/.dfm/.dpr",
        "extensions": [".pas", ".dpr", ".dpk", ".dproj", ".dfm", ".fmx", ".lpr", ".lfm", ".lpk", ".inc"],
        "keywords": "unit interface implementation initialization finalization uses program library type const var begin end procedure function class constructor destructor inherited override virtual abstract dynamic reintroduce overload property published private protected public default index read write stored try except finally raise on as is in of nil self result exit break continue with for to downto do while repeat until if then else case of array record set string ansistring widestring shortstring boolean integer cardinal longint shortint byte word smallint int64 single double extended real currency char widechar pchar pointer object packed forward external near far cdecl stdcall pascal register safecall message dispid",
    },
    "HTML": {
        "description": "markup HTML (4/5, XHTML) — tags, atributos, eventos comuns",
        "extensions": [".html", ".htm", ".xhtml", ".shtml"],
        "keywords": "html head body div span p a img ul ol li table thead tbody tfoot tr td th form input button select option textarea label fieldset legend script style link meta title nav header footer main section article aside figure figcaption picture source video audio canvas svg iframe template slot br hr code pre blockquote em strong b i u small sub sup mark del ins details summary dialog progress meter address cite q kbd samp output time data datalist optgroup colgroup col map area object embed param track wbr noscript base class id href src alt type name value style placeholder required disabled checked readonly target rel role tabindex onclick onchange oninput onsubmit onload onerror onfocus onblur onkeydown onkeyup onmouseover onmouseout charset content lang dir hidden contenteditable draggable spellcheck accept action method enctype autocomplete autofocus multiple size min max step pattern list srcset sizes loading async defer crossorigin integrity media",
    },
    "ASPRazor": {
        "description": "ASP.NET Razor (MVC + Blazor); cobre .cshtml/.razor/.vbhtml",
        "extensions": [".cshtml", ".razor", ".vbhtml"],
        "keywords": "page model inject implements inherits layout namespace attribute using addtaghelper removetaghelper taghelperprefix functions code section rendersection renderbody renderbodyasync html url viewdata viewbag tempdata model user request response if else for foreach while switch case do try catch finally throw await bind onclick onchange oninput onkeydown ref key typeparam preservewhitespace removetagwhitespace formaction asp-controller asp-action asp-route asp-for asp-area asp-fragment asp-handler asp-page asp-protocol asp-host environment names include exclude",
    },
    "ASPClassic": {
        "description": "ASP Classic (.asp, VBScript embarcado em <% %>); legado IIS",
        "extensions": [".asp", ".asa"],
        "keywords": "dim redim set let const sub function end if then else elseif case select wend while wend do loop until for next each in to step exit continue option explicit on error resume goto and or not xor mod is empty nothing null true false response request server session application cookies querystring form servervariables write redirect end clear flush buffer expires execute transfer createobject mapPath scripttimeout objectcontext err description number source raise vbscript jscript runat lcase ucase len mid left right trim ltrim rtrim chr asc cstr cint clng cdbl cdate cbool isnumeric isdate isnull isempty isobject isarray date now time year month day hour minute second weekday timer rnd randomize",
    },
    "ASPWebForms": {
        "description": "ASP.NET Web Forms (.aspx + controles asp:Tag, code-behind)",
        "extensions": [".aspx", ".ascx", ".master", ".ashx", ".asmx", ".axd"],
        "keywords": "page master control register import assembly reference output cache outputcache previouspagetype mastertype implements asp label button textbox literal image hyperlink linkbutton imagebutton dropdownlist listbox checkbox checkboxlist radiobutton radiobuttonlist gridview repeater datalist listview formview detailsview menu sitemappath treeview multiview view panel placeholder updatepanel scriptmanager scriptmanagerproxy contentplaceholder content validator requiredfieldvalidator regularexpressionvalidator rangevalidator comparevalidator customvalidator validationsummary loginview loginstatus loginname passwordrecovery changepassword createuserwizard wizard adrotator xml literal substitution timer runat id text visible enabled cssclass tooltip postbackurl validationgroup causesvalidation autopostback eventname commandname commandargument datasource datatextfield datavaluefield datakeynames itemtemplate alternatingitemtemplate headertemplate footertemplate edititemtemplate inserttemplate emptydatatemplate selectedrowstyle codebehind codefile language inherits theme stylesheettheme enableviewstate viewstate session application request response server",
    },
    "VBNet": {
        "description": "codigo fonte Visual Basic .NET (.vb)",
        "extensions": [".vb", ".vbs", ".bas"],
        "keywords": "addhandler addressof aggregate alias and andalso ansi as assembly auto binary boolean byref byte byval call case catch cbool cbyte cchar cdate cdec cdbl cdec class clng cobj const continue csbyte cshort csng cstr ctype cuint culong cushort date decimal declare default delegate dim directcast do double each else elseif end endif enum erase error event exit explicit false finally for friend from function get gettype global gosub goto group handles if implements imports in inherits integer interface into is isnot let lib like long loop me mod module mustinherit mustoverride mybase myclass namespace narrowing new next not nothing notinheritable notoverridable object of off on operator option optional or order orelse overloads overridable overrides paramarray partial preserve private property protected public raiseevent readonly redim rem removehandler resume return sbyte select set shadows shared short single static step stop string structure sub synclock then throw to true try trycast typeof uinteger ulong ushort using variant wend when where while widening with withevents writeonly xor yield",
    },
    "Clipper": {
        "description": "codigo fonte Clipper / dBase / xBase (.prg, .ch)",
        "extensions": [".prg", ".ch"],
        "keywords": "procedure function local public private static memvar parameters return external announce request set use index seek find skip goto top bottom locate continue go skip pack zap recall reindex append blank from while for if elseif else endif end do case otherwise endcase enddo while next exit loop begin sequence break recover end count sum average total replace with field record fields all the dbf ntx index on tag for evaluate macro nil true false dtoc ctod stod date time year month day str val len upper lower trim ltrim rtrim alltrim pad subs substr empty",
    },
    "Mumps": {
        "description": "codigo fonte M / MUMPS (Epic, VistA, GT.M, YottaDB)",
        "extensions": [".m", ".mumps", ".int"],
        "keywords": "set kill new do goto quit write read merge lock halt hang else if for break job xecu use view zwrite zsystem zload zsave open close $order $data $get $query $select $next $previous $find $piece $extract $length $char $ascii $translate $reverse $justify $fnumber $text $name $namespace $job $io $horolog $zhorolog $zversion $zdate $ztime $principal $device $key $test $stack $estack $ecode $reference $system $this $increment $listbuild $list $listget $listfind $listdata $listlength $listsame $random",
    },
    "CSS": {
        "description": "folhas de estilo CSS + pre-processadores (SCSS/SASS/LESS)",
        "extensions": [".css", ".scss", ".sass", ".less"],
        "keywords": "import media keyframes font-face supports namespace charset page document property layer container scope counter-style font-feature-values starting-style not hover focus active visited link target empty checked disabled enabled required optional invalid valid root first-child last-child nth-child nth-of-type first-of-type last-of-type only-child only-of-type before after first-letter first-line selection placeholder marker backdrop important inherit initial unset revert auto none display position top right bottom left float clear width height min-width min-height max-width max-height margin padding border outline background color font font-size font-family font-weight font-style line-height text-align text-decoration text-transform letter-spacing word-spacing white-space overflow visibility opacity z-index cursor transition animation transform translate rotate scale skew flex flex-direction flex-wrap justify-content align-items align-content align-self gap grid grid-template grid-template-columns grid-template-rows grid-area grid-column grid-row box-sizing box-shadow border-radius content list-style table-layout vertical-align block inline inline-block flex-block grid-block absolute relative fixed sticky static",
    },
    "XML": {
        "description": "documentos XML, XSD, XSLT, SVG, WSDL, POM, plist",
        "extensions": [".xml", ".xsd", ".xsl", ".xslt", ".svg", ".wsdl", ".pom", ".plist", ".rss", ".atom"],
        "keywords": "version encoding standalone xmlns xsi xsl xs xsd xslt cdata doctype entity element attribute attlist notation pi processing-instruction schema complextype simpletype sequence choice group all minoccurs maxoccurs annotation documentation include import redefine override union restriction extension list base type ref name namespace targetnamespace elementformdefault attributeformdefault nillable abstract substitutiongroup final block default fixed use required optional prohibited",
    },
    "JSON": {
        "description": "documentos JSON (puro, JSONC, JSON5, NDJSON/JSONL)",
        "extensions": [".json", ".jsonc", ".json5", ".ndjson", ".jsonl"],
        "keywords": "true false null",
    },
    "YAML": {
        "description": "documentos YAML (configs Kubernetes, Ansible, GitHub Actions, etc)",
        "extensions": [".yaml", ".yml"],
        "keywords": "true false null yes no on off",
    },
    "TOML": {
        "description": "documentos TOML (Cargo.toml, pyproject.toml, etc)",
        "extensions": [".toml"],
        "keywords": "true false inf nan",
    },
}


def load_syllables(path):
    """Le .drv -> lista de linhas que NAO sao header (#) nem keywords (=) nem vazias."""
    syls = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            s = line.rstrip("\n")
            t = s.strip()
            if not t or t.startswith("#") or t.startswith("="):
                continue
            syls.append(s)
    return syls


def header_block(name, description, extensions, base_label):
    """Monta as 4 linhas '#' de cabecalho padronizadas."""
    exts = " ".join(extensions) if extensions else "(fallback — sem extensao especifica)"
    return [
        f"# RAGnaRock driver: {name}",
        f"# descricao: {description}",
        f"# extensoes: {exts}",
        f"# base: {base_label}",
    ]


def driver_filename(name):
    """Convencao do nome: tokens_<Lang>_PTBR.drv; excecao: tokens_PTBR.drv (silabario base)."""
    return "tokens_PTBR.drv" if name == "PTBR" else f"tokens_{name}_PTBR.drv"


def write_base_driver(name, info, dry=False):
    """Regrava um driver BASE (PTBR/SourceCode) preservando suas silabas."""
    out = os.path.join(DRIVERS, driver_filename(name))
    syls = load_syllables(out)
    if dry:
        return out, len(syls), 0
    base_label = f"silabario proprio ({len(syls)} silabas, hand-managed)"
    header = header_block(name, info["description"], info["extensions"], base_label)
    with open(out, "w", encoding="utf-8") as f:
        for h in header:
            f.write(h + "\n")
        for s in syls:
            f.write(s + "\n")
    return out, len(syls), 0


def write_derived_driver(name, info, base_syls, dry=False):
    """Escreve um driver DERIVADO = SourceCode + keywords '=k'."""
    out = os.path.join(DRIVERS, driver_filename(name))
    syllables = len(base_syls)
    raw = [k.strip().lower() for k in info["keywords"].split() if k.strip()]
    seen, dedup = set(), []
    for k in raw:                                     # preserva ordem, sem dup
        if k not in seen:
            seen.add(k); dedup.append(k)
    if dry:
        return out, syllables, len(dedup)
    base_label = f"SourceCode ({syllables} silabas)  |  keywords atomicas: {len(dedup)} (linha '=palavra')"
    header = header_block(name, info["description"], info["extensions"], base_label)
    with open(out, "w", encoding="utf-8") as f:
        for h in header:
            f.write(h + "\n")
        for s in base_syls:
            f.write(s + "\n")
        for k in dedup:
            f.write("=" + k + "\n")
    return out, syllables, len(dedup)


def all_known():
    """Lista [(name, kind, info)] de todos os drivers conhecidos (base + derivados)."""
    out = [(n, "base", i) for n, i in BASE_DRIVERS.items()]
    out += [(n, "derived", i) for n, i in LANG_INFO.items()]
    return out


def main():
    ap = argparse.ArgumentParser(description="Gera drivers .drv (SourceCode + keywords + cabecalho expandido).")
    ap.add_argument("--only", action="append", default=None,
                    help="gera so a(s) linguagem(ns) listada(s); pode repetir o flag")
    ap.add_argument("--list", action="store_true", help="lista linguagens conhecidas e sai")
    ap.add_argument("--dry-run", action="store_true", help="nao grava; mostra o que seria feito")
    args = ap.parse_args()

    if args.list:
        for name, kind, info in all_known():
            n_kw = len(set(k.lower() for k in info.get("keywords", "").split())) if kind == "derived" else 0
            exts = " ".join(info["extensions"]) or "—"
            print(f"  [{kind:7s}] {name:14s} kw={n_kw:4d}  ext: {exts}")
        print(f"total: {len(BASE_DRIVERS)} base(s) + {len(LANG_INFO)} derivado(s) = {len(BASE_DRIVERS) + len(LANG_INFO)} drivers")
        return

    targets = all_known()
    if args.only:
        wanted = set(args.only)
        targets = [t for t in targets if t[0] in wanted]
        missing = wanted - {t[0] for t in targets}
        if missing:
            print(f"!! linguagens desconhecidas: {sorted(missing)}", file=sys.stderr)
            sys.exit(2)

    # carrega silabas de SourceCode (base p/ derivados) — uma so vez
    base_syls = load_syllables(BASE)
    print(f"base: tokens_SourceCode_PTBR.drv  ({len(base_syls)} silabas)")
    print(f"alvo: {len(targets)} driver(s)  ->  {DRIVERS}")
    print()
    for name, kind, info in targets:
        if kind == "base":
            path, syl, kn = write_base_driver(name, info, dry=args.dry_run)
        else:
            path, syl, kn = write_derived_driver(name, info, base_syls, dry=args.dry_run)
        flag = "DRY" if args.dry_run else "OK "
        tag = "BASE" if kind == "base" else "    "
        print(f"  [{flag}][{tag}] {os.path.basename(path):32s}  silabas={syl}  keywords={kn}  vocab={syl + kn}")


if __name__ == "__main__":
    main()
