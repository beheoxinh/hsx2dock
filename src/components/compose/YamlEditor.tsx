import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { EditorState, type Extension } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  dropCursor,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  HighlightStyle,
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentUnit,
  syntaxHighlighting,
} from "@codemirror/language";
import { yaml } from "@codemirror/lang-yaml";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import {
  autocompletion,
  completionKeymap,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { lintGutter, setDiagnostics, type Diagnostic } from "@codemirror/lint";
import { tags as t } from "@lezer/highlight";
import { composeSuggestions } from "../../lib/composeSchema";

export interface YamlEditorHandle {
  /** Moves the cursor to a 1-based line, scrolls it into view and focuses the editor. */
  goToLine: (line: number) => void;
}

export interface YamlEditorProps {
  value: string;
  onChange: (text: string) => void;
  /** Ctrl+S / Cmd+S. */
  onSave: () => void;
  /** Validation error: underlines this line with `errorMessage` as the tooltip. */
  errorLine?: number | null;
  errorMessage?: string;
  ariaLabel?: string;
}

const MONO = 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace';

// Zinc palette, square corners, transparent background so the parent's zinc-950 shows.
const theme = EditorView.theme(
  {
    "&": { height: "100%", backgroundColor: "transparent", color: "#f4f4f5", fontSize: "11px" },
    "&.cm-focused": { outline: "none" },
    ".cm-scroller": { fontFamily: MONO, lineHeight: "20px", overflow: "auto" },
    ".cm-content": { caretColor: "#e4e4e7", padding: "8px 0" },
    ".cm-line": { padding: "0 12px" },
    ".cm-cursor": { borderLeftColor: "#e4e4e7" },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(63,63,70,0.6)" },
    ".cm-activeLine": { backgroundColor: "rgba(39,39,42,0.4)" },
    ".cm-gutters": {
      backgroundColor: "rgba(24,24,27,0.5)",
      color: "#52525b",
      borderRight: "1px solid #27272a",
      borderLeft: "none",
    },
    ".cm-activeLineGutter": { backgroundColor: "transparent", color: "#a1a1aa" },
    ".cm-foldGutter .cm-gutterElement": { cursor: "pointer" },
    ".cm-matchingBracket": { backgroundColor: "rgba(113,113,122,0.45)", color: "inherit" },
    ".cm-searchMatch": { backgroundColor: "rgba(250,204,21,0.2)", outline: "1px solid rgba(250,204,21,0.4)" },
    ".cm-searchMatch-selected": { backgroundColor: "rgba(250,204,21,0.45)" },
    ".cm-selectionMatch": { backgroundColor: "rgba(113,113,122,0.3)" },
    ".cm-panels": { backgroundColor: "#18181b", color: "#e4e4e7", borderColor: "#27272a" },
    ".cm-panels-top": { borderBottom: "1px solid #27272a" },
    ".cm-textfield": {
      backgroundColor: "#09090b",
      border: "1px solid #3f3f46",
      borderRadius: "0",
      color: "#f4f4f5",
      fontSize: "11px",
      padding: "2px 6px",
    },
    ".cm-button": {
      backgroundImage: "none",
      backgroundColor: "#27272a",
      border: "1px solid #3f3f46",
      borderRadius: "0",
      color: "#e4e4e7",
      fontSize: "11px",
      padding: "2px 8px",
    },
    ".cm-tooltip": { backgroundColor: "#18181b", border: "1px solid #3f3f46", borderRadius: "0", color: "#e4e4e7" },
    ".cm-tooltip-autocomplete ul li[aria-selected]": { backgroundColor: "#3f3f46", color: "#fafafa" },
    ".cm-completionInfo": { backgroundColor: "#18181b", border: "1px solid #3f3f46", borderRadius: "0", fontSize: "11px" },
    ".cm-lintRange-error": { backgroundImage: "none", backgroundColor: "rgba(244,63,94,0.2)" },
    ".cm-tooltip-lint": { fontSize: "11px", whiteSpace: "pre-wrap" },
  },
  { dark: true }
);

const highlight = HighlightStyle.define([
  { tag: t.propertyName, color: "#7dd3fc" },
  { tag: [t.string, t.special(t.string)], color: "#86efac" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "#fcd34d" },
  { tag: t.comment, color: "#71717a", fontStyle: "italic" },
  { tag: [t.meta, t.keyword], color: "#c4b5fd" },
  { tag: [t.punctuation, t.separator, t.squareBracket, t.brace], color: "#a1a1aa" },
]);

/** Adapts the pure key/value suggestions to CodeMirror's completion source. */
function composeCompletionSource(ctx: CompletionContext): CompletionResult | null {
  const result = composeSuggestions(ctx.state.doc.toString(), ctx.pos);
  if (!result) return null;
  return {
    from: result.from,
    options: result.options.map((o) => ({
      label: o.label,
      apply: o.insert,
      info: o.description,
      type: o.kind === "key" ? "property" : "enum",
    })),
  };
}

/**
 * CodeMirror 6 editor for the compose file. Controlled through `value`:
 * edits flow out via `onChange`, external changes (revert, reload) are
 * pushed in without echoing back.
 */
export const YamlEditor = forwardRef<YamlEditorHandle, YamlEditorProps>(function YamlEditor(
  { value, onChange, onSave, errorLine = null, errorMessage = "", ariaLabel = "Compose file" },
  ref
) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // The listeners are created once, so they read the latest callbacks through refs.
  const onChangeRef = useRef(onChange);
  const onSaveRef = useRef(onSave);
  const syncingRef = useRef(false);

  useEffect(() => {
    onChangeRef.current = onChange;
    onSaveRef.current = onSave;
  });

  useEffect(() => {
    const extensions: Extension[] = [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      foldGutter(),
      history(),
      drawSelection(),
      dropCursor(),
      // Single cursor only: the rest of the app has no multi-select.
      EditorState.allowMultipleSelections.of(false),
      indentUnit.of("  "),
      EditorState.tabSize.of(2),
      bracketMatching(),
      highlightSelectionMatches(),
      yaml(),
      syntaxHighlighting(highlight),
      search({ top: true }),
      lintGutter(),
      autocompletion({ override: [composeCompletionSource], activateOnTyping: true }),
      keymap.of([
        {
          key: "Mod-s",
          preventDefault: true,
          run: () => {
            onSaveRef.current();
            return true;
          },
        },
        ...completionKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...foldKeymap,
        indentWithTab,
        ...defaultKeymap,
      ]),
      theme,
      EditorView.contentAttributes.of({ "aria-label": ariaLabel }),
      EditorView.updateListener.of((update) => {
        if (update.docChanged && !syncingRef.current) onChangeRef.current(update.state.doc.toString());
      }),
    ];
    const view = new EditorView({
      parent: hostRef.current!,
      state: EditorState.create({ doc: value, extensions }),
    });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // `value` is read once here; later changes are pushed by the effect below.
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === value) return;
    syncingRef.current = true;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } });
    syncingRef.current = false;
  }, [value]);

  // Diagnostics map through later edits, so they only need refreshing when the error itself changes.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const diagnostics: Diagnostic[] = [];
    if (errorLine !== null) {
      const doc = view.state.doc;
      const line = doc.line(Math.min(Math.max(errorLine, 1), doc.lines));
      diagnostics.push({ from: line.from, to: line.to, severity: "error", message: errorMessage });
    }
    view.dispatch(setDiagnostics(view.state, diagnostics));
  }, [errorLine, errorMessage]);

  useImperativeHandle(
    ref,
    () => ({
      goToLine(line: number) {
        const view = viewRef.current;
        if (!view) return;
        const doc = view.state.doc;
        const target = doc.line(Math.min(Math.max(line, 1), doc.lines));
        view.dispatch({
          selection: { anchor: target.from },
          effects: EditorView.scrollIntoView(target.from, { y: "center" }),
        });
        view.focus();
      },
    }),
    []
  );

  return <div ref={hostRef} className="absolute inset-0" />;
});
