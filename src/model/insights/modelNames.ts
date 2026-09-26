/**
 * Model names across sources. Transcripts use ids (`claude-opus-5-5`, `gpt-5.6-sol`), Epoch AI uses
 * display names (`Claude Opus 5`, `GPT-5.6 Sol`) and Arena uses its own names with the reasoning
 * effort attached (`claude-opus-5.5-high`, `GPT 6 Astra (Max)`). `modelKey` only folds spelling
 * (case, spaces, dots, dashes, brackets), so two sources join only when they name exactly the same
 * model. An effort variant stays its own entry; `effortVariant` recognises one so it can be shown
 * next to its model under its own name, never merged into it.
 */

export function modelKey(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[\s._/()]+/g, "-")
    .replace(/[^a-z0-9-]/g, "")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

/** The reasoning-effort suffixes Arena appends to a model's name. */
export const EFFORT_LEVELS = ["minimal", "low", "medium", "high", "xhigh", "max"] as const;
export type EffortLevel = (typeof EFFORT_LEVELS)[number];

/** The effort of `candidate` when it is exactly `base` plus one effort suffix, otherwise `null`. */
export function effortVariant(baseKey: string, candidateKey: string): EffortLevel | null {
  if (!candidateKey.startsWith(`${baseKey}-`)) return null;
  const suffix = candidateKey.slice(baseKey.length + 1);
  return (EFFORT_LEVELS as readonly string[]).includes(suffix) ? (suffix as EffortLevel) : null;
}

const UPPER_WORDS = new Set(["gpt", "glm"]);

function isVersion(token: string): boolean {
  return /^\d+(\.\d+)?$/.test(token);
}

/** A transcript model id as people write it: `claude-opus-5-5` → `Claude Opus 5.5`. */
export function displayModel(id: string): string {
  const slash = id.lastIndexOf("/");
  if (slash > 0) return `${id.slice(0, slash)}/${displayModel(id.slice(slash + 1))}`;
  if (/^o\d/.test(id)) return id;
  const tokens = id.split(/[-_]/).filter(Boolean);
  const words: string[] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    const previous = words.at(-1);
    const joinsPrevious = previous !== undefined && UPPER_WORDS.has(previous.toLowerCase()) && /^\d/.test(token);
    if (isVersion(token)) {
      let version = token;
      while (!version.includes(".") && version.length === 1 && /^\d$/.test(tokens[index + 1] ?? "")) {
        version += `.${tokens[index + 1]}`;
        index += 1;
      }
      if (joinsPrevious) words[words.length - 1] = `${previous}-${version}`;
      else words.push(version);
    } else if (joinsPrevious) {
      words[words.length - 1] = `${previous}-${token}`;
    } else if (UPPER_WORDS.has(token.toLowerCase())) {
      words.push(token.toUpperCase());
    } else {
      words.push(token.charAt(0).toUpperCase() + token.slice(1));
    }
  }
  return words.join(" ") || id;
}
