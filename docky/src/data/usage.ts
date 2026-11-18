import claude from "@/assets/usage/claude.svg";
import codex from "@/assets/usage/codex.svg";
import copilot from "@/assets/usage/copilot.svg";
import gemini from "@/assets/usage/gemini.svg";
import opencode from "@/assets/usage/opencode.svg";

export interface QuotaExample {
  label: string;
  used: number;
  resets: string;
}

/** One supported AI client with illustrative, not live, quota values. */
export interface UsageProvider {
  name: string;
  logo: ImageMetadata;
  color: string;
  primary: QuotaExample;
  secondary?: QuotaExample;
}

export const usageProviders: UsageProvider[] = [
  {
    name: "Claude Code",
    logo: claude,
    color: "#d97757",
    primary: { label: "5 h", used: 46, resets: "Resets 3:10 PM" },
    secondary: { label: "7 d", used: 35, resets: "Resets Nov 21" },
  },
  {
    name: "Codex",
    logo: codex,
    color: "#87bef5",
    primary: { label: "5 h", used: 28, resets: "Resets 4:30 PM" },
    secondary: { label: "7 d", used: 41, resets: "Resets Nov 21" },
  },
  {
    name: "GitHub Copilot",
    logo: copilot,
    color: "#93a4ba",
    primary: { label: "Premium", used: 32, resets: "Resets Nov 30" },
    secondary: { label: "Chat", used: 74, resets: "Resets Nov 30" },
  },
  {
    name: "OpenCode Go",
    logo: opencode,
    color: "#84c4a6",
    primary: { label: "5 h", used: 25, resets: "Resets 4:00 PM" },
    secondary: { label: "7 d", used: 40, resets: "Resets Nov 21" },
  },
  {
    name: "Gemini / Antigravity",
    logo: gemini,
    color: "#6f92ff",
    primary: { label: "Gemini Pro", used: 49, resets: "Resets 5:10 PM" },
    secondary: { label: "Gemini Flash", used: 18, resets: "Resets Nov 21" },
  },
];

export const usageIntro = {
  eyebrow: "AI usage",
  title: "AI plan usage, at a glance.",
  lead: "Docky shows available limits for Claude Code, Codex, GitHub Copilot, OpenCode Go, and Gemini CLI or Antigravity. Gemini Pro and Flash quotas appear separately when available.",
} as const;

export const usageFacts = [
  {
    title: "Only while you use them",
    body: "The island recognizes five clients: Claude Code, Codex, GitHub Copilot, OpenCode Go, and Gemini CLI or Antigravity.",
  },
  {
    title: "Your existing session",
    body: "Docky reuses the sign-in those tools already keep on your PC. Gemini Pro and Flash are shown as separate model families when the provider supplies those quotas.",
  },
  {
    title: "Straight to the provider",
    body: "It checks available limits every five minutes or when you press Refresh. Antigravity is queried through its authenticated local service; a separate process keeps the dock responsive.",
  },
  {
    title: "One switch to turn it off",
    body: "Settings → Menu bar → Show AI provider usage. Docky stores no credentials.",
  },
] as const;
