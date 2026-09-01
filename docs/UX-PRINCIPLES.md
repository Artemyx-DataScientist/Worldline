# Worldline UX Principles

## Web applications, not tabs

Worldline does not organize the Internet around isolated pages and browser tabs.
Modern sites are often full cloud applications, so the top-level user-facing
entity is a **WebApp**, while individual pages remain subordinate views or
surfaces inside that application.

Examples:

- GitHub may contain a repository, issue, pull request, and Actions view.
- ChatGPT may contain several conversations and library views.
- X, MAX, imageboards, news sites, and other web software each appear as one
  application at the top level, even when several internal views are open.

The exact visual form of those internal views is intentionally not fixed yet.
They do not need to look like conventional browser tabs.

## Content first

The current content or application should occupy almost all available space.
Worldline control surfaces are secondary to content and should avoid reproducing
conventional browser chrome merely because users are familiar with it.

The primary shell controls are placed at the **bottom** of the interface:

- open WebApps are represented primarily as application icons rather than tab
  rectangles;
- address/navigation input lives in the bottom control layer;
- search/command input is associated with that same bottom area;
- the final geometry, composition, and interaction model of these controls remain
  open for iteration.

Fullscreen is therefore a normal working state rather than a special mode for
video or presentations: content keeps the screen, while navigation and control
remain compact, transient, or revealable when needed.

## Assistant as a side surface

The AI assistant remains available as a side surface because persistent
page-aware conversation, explanation, research, and action control are useful.
It is a companion to the current context, not the primary organizing metaphor of
the browser and not a reason to structure the entire UI around chat.

## UX grouping is not a security boundary

A WebApp is a semantic UX grouping. It does **not** automatically merge origins,
BrowserContexts, cookies, permissions, storage, principals, or capability
authority. Application identity and security authority remain separate concepts.

## Guiding statement

> **Worldline presents the Internet as a set of applications and contexts, not a
> collection of tabs. Content is primary; browser chrome is secondary and
> transient.**

This document fixes the product principle, not the final shell layout. The visual
form may change substantially as the Worldline shell is designed, provided these
semantics are preserved.
