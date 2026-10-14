# Using BoBe

What BoBe does day to day, and how to shape her. If she is not installed yet,
start with [Getting started](getting-started.md).

<p align="center">
  <img src="media/app-conversation.png" alt="BoBe's conversation panel beside her avatar" width="720">
</p>

## Contents

- [The overlay](#the-overlay)
- [Talking to BoBe](#talking-to-bobe)
- [When BoBe speaks first](#when-bobe-speaks-first)
- [Goals](#goals)
- [Memory, profiles and souls](#memory-profiles-and-souls)
- [Screen awareness](#screen-awareness)
- [Tools and work](#tools-and-work)
- [Settings at a glance](#settings-at-a-glance)
- [Privacy](#privacy)
- [Keeping costs in hand](#keeping-costs-in-hand)

## The overlay

BoBe is a small face on your desktop. Everything starts there.

| You do | BoBe does |
|---|---|
| Click her face | Opens the conversation, ready to type. Click again to close it |
| Move the pointer near her | Shows the chat, microphone and Settings controls on her rim |
| Drag her face | Moves her. The conversation opens toward the side with room |
| Click anywhere else | Folds the conversation away, unless you are part-way through writing. A recent answer stays as a small bubble |
| Press Escape | Dismisses the conversation completely |
| Scroll up in the conversation | Shows earlier messages |

She never takes the keyboard by herself. It moves to her when you open the
conversation and goes back to the app you were using when you close it. The
empty space around her does not block clicks to the windows underneath.

<img src="media/app-thinking.gif" alt="BoBe's ring glows and a comet orbits while she thinks" width="150" align="right">

While she thinks or uses a tool, her rim glows, her ring becomes an orbiting
comet, and a small badge names the kind of tool. During a task the send button
becomes **guide** or **stop**: guide her without starting over, or stop her.

The menu bar icon shows or hides her, opens Settings, and quits. Its **More**
menu has the interface language, updates, physical BoBe setup and About.

## Talking to BoBe

Click the microphone on her rim and speak. She answers out loud and shows the
words as well.

<p align="center">
  <img src="media/app-settings-voice.png" alt="Voice settings" width="720">
</p>

**Where voice runs.** Speech recognition runs on your Mac. BoBe's speaking
voice runs wherever her daemon runs, or on the Mac if you choose the
experimental in-app voice. Neither uses a cloud speech service, and raw audio
is not stored. The words you say are sent to your model like anything you type.

**Speech models.** English needs about 600 MB. Another language adds about
0.7 GB. Install them during setup or from **Settings → Voice**.

**Interrupting.** Just talk over her.

- She turns her volume down while you overlap.
- A short acknowledgement such as "yeah", "mm-hmm" or "got it" does not stop
  her.
- "Wait", "no", "actually" and similar openers stop her on the first word.
  Anything else stops her after two words.
- She then knows which sentences you heard in full and which one was cut off,
  so she does not assume you heard the rest.

Stopping her from the keyboard, or by typing a new message, works too, but
she is then not told how far you had listened.

**Pausing mid-sentence.** If you trail off on a word like "and" or "so", she
waits a moment for the rest instead of answering half a thought.

**Settings → Voice** has:

| Setting | What it changes |
|---|---|
| Enable voice | Hides the microphone and stops listening when off |
| Language | The language she listens for. English is the most reliable; others are still maturing |
| Pause sensitivity | How long she waits after you stop. Patient waits longer, Tight answers sooner |
| BoBe's voice | Which voice she speaks with |
| Speed | 0.5× to 2.0× |
| Show partial caption | A live transcript while you speak |

The interruption and trailing-word rules know English words. In other
languages they fall back to counting words and punctuation.

## When BoBe speaks first

BoBe can start a conversation: a scheduled check-in, something she noticed, or
the result of work she finished. These arrive as a small bubble beside her
face, labelled **Thought** or **Finished**, with one-tap replies such as
**Reply**, **Tell me more** and **Not now**.

A message you did not get to is not lost. It waits, her face shows a count,
and hovering over her brings the newest one back. Opening the conversation
clears the count. Waiting messages are kept by the app while it runs; the
conversation itself is always saved.

How often she speaks first is yours to set in **Settings → Behavior**:

<p align="center">
  <img src="media/app-settings-behavior.png" alt="Behavior settings: Quiet, Balanced and Proactive" width="720">
</p>

| Mode | Screen context | Scheduled check-ins | Goals |
|---|---|---|---|
| **Quiet** | Off | Off | Keep working |
| **Balanced** | Occasional | Off | Keep working |
| **Proactive** | More often | On | Keep working |

Each part can also be set by itself: screen capture, check-in times, how
conversations are managed, and how often goals are reviewed.

## Goals

A goal is something you want to get done, written down so BoBe can help with
it between conversations. Create one in **Settings → Goals**, or just tell her
about it. She may also suggest and keep goals herself when something clearly
matters to you.

Each goal is a plain Markdown file in `~/.bobe/goals/` with a title, a status,
a priority and a few sections: a summary, why it matters, and notes. Edit it in
Settings or in any editor and she picks up the change.

**How she works on them.** Active goals are reviewed every hour by default. In
a review she decides what would actually help, does it quietly, and records
what happened. A separate, smaller decision then chooses whether to tell you
now, keep it for later, or say nothing. A routine technical failure does not
turn into a question for you.

**Every goal ends.**

- When a goal reaches its ending point, she marks it complete.
- When it cannot be achieved, or nothing useful can happen until you act, she
  pauses it and records why.
- If three reviews in a row change nothing, the reviews rest. They start again
  when the goal changes, for example when you edit it or add a note.

You can **Pause**, **Complete** or **Archive** any goal yourself. New installs
include one example goal, which starts paused.

Goal reviews happen in the daemon. If the Mac app started the daemon, they stop
when you quit the app. If you installed BoBe as a service from the
[command line](cli.md), they continue with the app closed.

## Memory, profiles and souls

<p align="center">
  <img src="media/app-settings-home.png" alt="Settings home: how to teach BoBe" width="720">
</p>

| | What it is | Where to change it |
|---|---|---|
| **Memory** | One document of lasting facts about you and your work. BoBe adds to it as she learns | **Settings → Memories**, or `~/.bobe/memory.md` |
| **User profiles** | Who you are and how you like things done. Each profile can be turned on or off | **Settings → User Profiles** |
| **Souls** | BoBe's personality: how she talks and what she cares about | **Settings → Souls** |

You can read all of it and change any of it. Nothing about you is kept that you
cannot see.

BoBe keeps a separate conversation thread for each place you meet her: the Mac
app, the command line, and each physical body. They all share the same memory,
goals and skills. She recalls an earlier conversation from another thread when
it is relevant instead of copying every transcript everywhere.

## Screen awareness

Screen awareness lets BoBe understand what you are working on. It is optional
and off in Quiet mode.

When it is on, about every two minutes:

1. BoBe asks the Mac app to look, and the app captures the main display.
2. The text on it is recognised on the Mac.
3. The picture is discarded.
4. The recognised text, at most 6,000 characters, goes to your small model
   with the name of the app in front and its window title, and comes back as
   one sentence of context.

This works the same when BoBe lives on another computer: the Mac reads its own
screen and sends her only the text. While the app is closed she sees nothing.

No picture is sent to any model. For the same reason, images, video and
drawings without text are not described.

Turn it on or off in **Settings → Behavior**. It needs the Screen Recording
permission for the BoBe app.

## Tools and work

BoBe can do things, not only talk about them:

- read, write and search files
- run shell commands and use Git
- call tools from MCP servers you connect
- follow skills, and write a new skill once she has proved a procedure works
- run several tool calls at once and hand part of a job to a helper agent

She works in one folder, `~/.bobe/workspace/`, so you can always see what she
made.

**This is not a sandbox.** Her tools run as your user account, with your
permissions. Turning a tool off in **Settings → AI & tools** removes it from
her, but other enabled tools can still reach your files. Connect only MCP
servers you trust.

**Longer work.** She can keep working on a task after she has answered, and
return to something on a schedule. Schedules keep their times across restarts.
Work that a restart interrupted is not blindly replayed: she looks at what
already happened first, and you can resume or cancel a task yourself. Ask her
in plain words ("look at this again tomorrow morning") or manage it under
**Settings → AI & tools**.

**Questions that matter.** When she needs a decision only you can make, she
saves the exact question and waits, without holding anything else up. Answer in
the conversation, by voice, or with the question's own buttons.

## Settings at a glance

Open Settings from her rim or the menu bar icon.

| Group | Section | What is there |
|---|---|---|
| **BoBe** | Souls | Her personality |
| | Appearance | Colour themes, and faces for the physical body |
| | Voice | Language, voice, speed, and speech models |
| **You** | User Profiles | Who you are |
| | Goals | What she works toward with you |
| | Memories | What she remembers |
| **Initiative** | Behavior | How present she is: screen context, check-ins, goal reviews |
| | Privacy | What is stored, where it goes, and how to remove it |
| **Under the hood** | AI & tools | Where her AI runs, which model does which kind of work, activity, and her tools |
| | Tools | Tools and services she can use; connect MCP servers here |
| | Runtime | Where BoBe runs and how this Mac reaches her |
| | Expert mode | Power-user controls such as technical model names, raw tool configuration and strict offline |

<p align="center">
  <img src="media/app-settings-appearance.png" alt="Appearance settings: themes and body faces" width="720">
</p>

The interface is available in English, Greek, Chinese, German, Spanish,
Portuguese, Korean, Japanese and French.

## Privacy

- **Local first.** Conversations, memory, goals, skills and files live under
  `~/.bobe/` on the machine that runs BoBe's daemon.
- **What leaves.** When you use a cloud model, the context for that answer goes
  to that provider. **Settings → Privacy** lists where each kind of work is
  sent. With a local model, your conversations stay on that machine. Tools
  you enable can still use the network.
- **Screens.** Only recognised text is sent, never a picture.
- **Voice.** Recognised and spoken on your own hardware. Raw audio is not
  stored.
- **Diagnostics.** Optional. If you turn them on, BoBe keeps up to 30 days of
  local performance summaries and never uploads them by herself.
- **Deleting.** **Settings → Privacy** can delete conversations, memory, goals,
  learned skills, workspace files and her session state. Provider credentials,
  installed models and backups you made are kept.

## Keeping costs in hand

A cloud provider bills for every request. BoBe makes requests when you talk to
her, and also on her own:

| What | When |
|---|---|
| Your conversations | When you write or speak |
| Goal reviews | Hourly for each active goal, until its reviews rest |
| Update decisions and summaries | After goal work and other background results |
| Screen context | About every two minutes while screen awareness is on |
| Check-ins | At the times you set, if check-ins are on |

Goal work uses the goals model. Update decisions, summaries and screen context
use the small model.

To spend less:

- Choose **Quiet** in **Settings → Behavior**.
- Pause or complete goals you are not working on. Paused goals are not
  reviewed.
- Give the small work to a small, inexpensive model in
  **Settings → AI & tools**.
- Use a local model.
- Quit BoBe when you do not want her working. A daemon the app started stops
  with the app.
