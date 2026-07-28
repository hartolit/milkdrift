# Vision

I started this project because I think the way we currently connect AI to computers is still strangely primitive.

We have models capable of reasoning across huge amounts of information, yet when we want them to actually *do* something, we often reduce the interaction to a giant pile of text: JSON schemas, HTTP APIs, JavaScript wrappers, tool descriptions, serialized state, verbose responses, and increasingly high-level actions that have to be explained to the model before it can use them.

It works, but I don't think it is where this ends.

The model's context is one of the most valuable resources in the entire system. Every tool description, protocol field, repeated state object, irrelevant search result, and implementation detail competes with the actual problem the model is trying to reason about. I want to treat that context as something almost sacred: keep it clean, dense, and focused on the information that matters.

That is the idea underneath this project.

Not just another local LLM interface, and not an attempt to reimplement everything that Candle, llama.cpp, or other inference engines already do well. I want to build the foundations for a local AI system where inference, tools, context, and the computer itself can eventually fit together much more naturally.

For now, it starts as an inference app. That is intentional.

## The part that bothers me

A lot of modern AI tooling feels like we are bolting intelligence onto software that was designed for something else.

The model sits at the top, and underneath it is layer after layer of abstractions made for humans, web applications, distributed services, or conventional programs. To search the web, inspect a file, query a database, compile code, or interact with the operating system, the model often needs a textual description of some API, then has to produce another textual representation of a call, which gets translated through several layers before anything actually happens.

Then the result comes back as more text.

Sometimes this is exactly the right design. HTTP and JSON are popular for good reasons. Standard interfaces make systems interoperable, inspectable, and easy to build around.

But I don't think they should automatically become the language between an AI and the machine it lives on.

If a model is running locally, why should every internal capability pretend to be a web service? Why should a small, well-defined operation need a giant schema injected into context? Why should the primary reasoning model read through fifty search results when something faster could reduce them to the few pieces that matter? Why should it reconstruct system state from prose when the runtime already knows that state exactly?

Those questions are more interesting to me than building another chat window.

## Keep the context clean

The clearest example is web search.

If a prompt depends on information that may have changed, I would rather let a small, fast agent inspect and expand it *before* the expensive model sees it. That agent can search, compare sources, throw away noise, keep the useful evidence, and hand the main model a compact piece of current context instead of the raw internet.

The main model should still be able to ask for another search when its own reasoning exposes a missing fact or a new direction. I just don't want that to be the ordinary path. Retrieval is often preparation for reasoning, not something the largest model should spend its time orchestrating from scratch.

The same idea could apply elsewhere. A code-search agent might understand an indexed repository without feeding the primary model thousands of unrelated lines. A compiler integration could reduce pages of diagnostics into the failures relevant to the current change while keeping the raw evidence outside the active context. A filesystem capability could expose a narrow operation without teaching the model an entire shell environment. A system service could keep exact internal state and reveal only the parts needed for the current decision.

Modern AI systems already use pieces of this idea through retrieval, subagents, context compression, and tools that keep intermediate work outside the main conversation. That validates the problem, but it does not settle it for me. I am interested in whether those ideas can become native parts of a local runtime instead of depending on another large orchestration layer around an otherwise conventional software stack.

I don't know yet what the right abstraction is. It might involve typed handles, capability-based APIs, compact binary representations, specialist models, deterministic processors, or something I haven't encountered yet.

The important part is the direction: **the model should reason about meaning, not spend half of its attention translating plumbing.**

## The prompt should be part of a workflow

I increasingly think the path from prompt to output should be something the user can shape rather than one hard-coded sequence.

A request might first pass through a small research agent that adds current information, a project agent that retrieves the relevant code and decisions, or a deterministic processor that attaches exact machine state. The primary model then receives a prompt that has already been prepared for the problem in front of it.

Work can continue while the answer is being generated. A small model could watch the live output for a narrow concern without taking over the main response. On a software project it might notice that a proposed change is pushing everything into one giant module, that the workspace is accumulating pointless crates, or that the response is becoming vague and repetitive. Another workflow might enforce a writing convention, check claims against available evidence, or look for a contradiction with the context that was already established.

The useful response to that signal does not always have to be the same. A corrective workflow might fix a small semantic mistake locally. A larger problem could become a new instruction for the primary model, giving it a chance to reconsider what it is doing. Some checks may be better after generation, when the whole result is available. Others can run beside it and react before the answer is finished.

That also means I don't want to treat a conversation as an append-only transcript that becomes increasingly expensive to carry forever. The raw history can remain available for provenance, but the active context should be allowed to improve its representation of the past. A wrong assumption can be retracted. Repeated explanations can become one stronger statement. A stale summary can be replaced. If the model discovers that an earlier part of the conversation was misleading, the useful state should be able to reflect the correction instead of preserving the mistake simply because it happened first.

Bad context has a cascade effect. Once sloppy language, duplicated assumptions, or incorrect state enters the working window, later generations inherit it as if it were evidence. The opposite should also be possible: a system that notices problems, repairs what it carries forward, and gradually makes the working context denser rather than noisier.

Some workflows may not exist to change the answer at all. A small trigger agent could watch for a user-defined condition and invoke an external capability while generation continues. If the user seemed a bit down, a configured workflow might start some music or send a simple message to a friend; another could update a service or react to something happening in a project. Those actions should be explicit about the authority they have and easy to disable, but they do not need to wait for the primary model to finish reasoning if the relevant signal is already clear.

I don't want any of these examples to become *the* pipeline. One combination may dramatically improve a model while another makes it worse. A useful system should make stages easy to add, remove, bypass, reorder, run in parallel, or feed back into earlier state without forcing every conversation through the same machinery.

That flexibility may be more valuable than any individual workflow I can design now. Instead of depending on an external service every time someone wants a different orchestration pattern, the runtime itself could become a place where these ideas can be assembled and tested locally.

The danger is obvious: flexibility can turn into a maze of hidden behavior, latency, duplicated work, and features that are difficult to reason about. The system has to make composition possible without making complexity permanent. A workflow should earn its place in exactly the same way as any other abstraction in the project.

## Memory as a moving window

Another idea came from a cellular automata game I built.

The world was divided into chunks, with a toroidal grid acting as the bounded active area around the player. As the player moved, the active window moved with them. Chunks could be loaded into and removed from that window, while the larger world remained intact outside it.

The grid was finite. The world it represented was not limited to whatever happened to be visible at that moment.

That feels surprisingly close to the problem of long-term context.

Imagine we spend an hour talking about spaceships and then drift into philosophy. As the conversation grows, the spaceship discussion will eventually be compressed, dropped, or require a deliberate search before it becomes useful again.

What if the conversation did not disappear because we changed subjects? What if we had simply moved somewhere else?

The long-term memory could remain outside the model as durable contextual chunks, while the context window acts as a moving projection over them. When the conversation moves into philosophy, the active window follows. If we later return to propulsion, hull design, or something strongly connected to that earlier discussion, the system could move back toward the relevant region and bring those chunks into view again.

The model would still have a finite context window. Context overflow just would not have to mean that the underlying memory was gone.

This also makes me think about more than one memory space. One could organize personal memories, another could follow a project, while others index books, research papers, code, or temporary web research. The same information might appear through several views depending on whether the useful relationship is chronological, semantic, causal, personal, or project-specific.

The hardest part is giving those chunks meaningful positions.

A game world already has coordinates. A conversation does not. Two memories might be close because they happened together, discuss the same subject, depend on each other, came from the same source, or tend to become useful at the same time.

That may mean a literal two-dimensional toroidal grid is the wrong structure. It may only be the analogy that points toward something more useful: memory as a navigable world, with a bounded working window that moves through it.

I find that idea more compelling than treating long-term memory as one enormous transcript or a flat bag of embeddings.

It could also change how useful smaller local models are. A small model will not become a frontier model just because it has good retrieval, but it may become far more capable when the surrounding system consistently places it near the right information.

The goal would not be an infinite prompt. It would be a finite prompt with access to a much larger world.

The model would not carry the library.

It would move through it.

## A local AI system, not just a model runner

This is where the project starts becoming more ambitious.

I don't really want to build an application where the UI is the product and the inference engine is the interesting part underneath. I want the system around the model to become interesting too.

The inference backend should eventually be replaceable. Candle might be right for one path, llama.cpp for another, and something else might make more sense later. The project should own the behavior that matters above those implementations: lifecycle, context, memory, tools, permissions, state, orchestration, and the way the AI experiences the rest of the machine.

That is also how I think about `application-runtime`. Its purpose is not to become a giant desktop backend. It is the reusable layer where application behavior can exist without belonging to Slint, a terminal, a web page, or a particular inference implementation. The presentation should sit above it; model execution should sit below it.

I want to understand what current inference stacks are doing, where they waste time or memory on local machines, and which parts can be shaped differently when the surrounding system is designed for local AI rather than as a general-purpose framework. I may discover that most of the underlying work is already better solved elsewhere, which would be useful too.

The point is not to replace mature libraries for the sake of owning more code. It is to get close enough to the machine that I can tell which layers are essential and which ones only exist because that is how software has traditionally been assembled.

## One system across several machines

Local does not have to mean that every model runs inside the same computer.

A desktop might have enough memory for the large model I actually want to talk to but no room left for the smaller agents that search the web, inspect a repository, rerank context, or watch a live response. Meanwhile an old laptop or unused PC might have more than enough resources for those jobs.

I want those machines to be able to become one local AI system.

The large model can stay where the memory and compute make sense. Specialist work can run elsewhere. A node should be able to expose what it can do and what resources it has available, while the runtime decides where a particular piece of work belongs. The user should not have to think about a second machine as a separate application every time a research agent is needed.

There will also be models that do not make sense on any machine I own. Local-first should not mean refusing useful compute elsewhere. I might rent a GPU for a large model, connect a machine in another location, or deliberately use a hosted model such as OpenAI, Claude, or Gemini for work that local hardware cannot handle.

I want that to feel like another execution choice rather than a different application. The conversation, context, workflows, and tools should not have to be rebuilt around the provider. At the same time, crossing that boundary should never be invisible. If context leaves my machines, the system should know where it is going, which capabilities change, and what authority that target has.

I also don't want this project to become a VPN. WireGuard, Tailscale, NetBird, an ordinary LAN, or whatever comes next already solve the problem of making machines reachable. If two nodes can communicate, this project should care about discovering or naming the peer, establishing identity, understanding its capabilities, and routing work to it. The network underneath is someone else's problem unless there is a very good reason to make it ours.

This changes how I think about the application itself. A Slint desktop interface is useful because it gives the project a tangible product while the runtime is still taking shape. A btop-like terminal interface could make the same system pleasant to operate over SSH on a server, an old laptop, or a headless machine. A web interface might later make conversations reachable from devices that should not run the inference stack themselves.

None of those interfaces should *be* the node.

Closing a terminal window should not define the lifetime of a server that is meant to keep serving work. A desktop client should be able to control local state without making that state fundamentally graphical. The same application behavior should be reachable from different frontends, and eventually a frontend may talk to a local node, a remote one, or a collection of them.

I like the idea that installation can still feel simple. Run the binary, choose a model, inspect the machine, see peers, change settings, start or stop services, and understand what the system is doing without having to remember a collection of commands and configuration files. Whether that experience is presented through Slint or a terminal matters less than keeping the underlying runtime independent of both.

A mesh also gives small models a more concrete role. They do not only save context; they can occupy machines where the primary model would never fit alongside them. The system becomes capable by combining hardware that would otherwise sit idle, without pretending that every task belongs on the biggest box.

## The larger idea

I have another project sitting on the side: a custom x86_64 operating system built around an ECS-like architecture.

The original thought was simple: if an AI were a first-class participant in an operating system rather than an application bolted onto one, what would the machine look like?

Would it still expose itself through the same collection of files, processes, command lines, configuration formats, services, and protocols? Or could the system present a much cleaner model of what exists, what owns what, what can be changed, and what each capability is allowed to do?

An ECS-style system is interesting to me because it can make the machine look less like a pile of opaque programs and more like a world of explicit state and capabilities. That feels naturally compatible with an AI that needs to understand and manipulate a system without being given unrestricted access to everything.

I don't know if that idea is good yet.

I've put the OS aside because I don't want to disappear into years of low-level implementation before understanding whether the premise is even useful. This inference project is a much better place to learn. It can become a real application, expose the actual pain points, and still leave me with something worthwhile if the larger idea turns out to be wrong.

That matters to me. I don't want the grand vision to become an excuse to build nothing.

## Uniqueness, standardization, and security

One of the stranger ideas behind the OS project is a suspicion that extreme standardization may become a bigger security problem as automated attackers become more capable.

Today, millions of machines often run the same operating systems, libraries, services, protocols, and deployment patterns. That standardization is incredibly useful, but it also means one successful exploit can sometimes be turned into a repeatable recipe.

Now imagine attackers that can reason, adapt, probe, generate exploits, and operate continuously at machine speed. The attacker is no longer one person patiently studying one target; it can become a huge population of automated systems constantly testing everything they can reach.

In that world, I wonder whether some amount of system-level uniqueness becomes valuable.

Not "security through obscurity" in the old sense. Hiding a broken system does not make it secure. A unique buffer overflow is still a buffer overflow.

What interests me is the possibility of systems that share strong security principles without sharing the exact same internal structure. Two machines might agree on how to communicate while composing capabilities, layouts, services, execution paths, and internal relationships differently.

Then compromising one machine does not automatically reveal a perfect map of the next million.

I have no proof that this would work. It could easily create new problems: harder auditing, harder updates, harder reproducibility, and a much larger space for subtle mistakes. But I think the question is worth asking.

The future probably needs both: common ground where communication and verification matter, and diversity where identical internal structure only makes large-scale exploitation easier.

## Trust should probably feel more human

The same uncertainty applies to communication between AI systems.

I don't like the idea that authentication automatically means trust.

People don't usually meet someone and immediately hand over their private history, personal thoughts, credentials, and authority to act on their behalf. Trust develops slowly and remains contextual. You might trust someone with one thing and not another, even after knowing them for years.

A personal AI holding a large amount of information about its user may need something closer to that.

Two systems could begin with almost nothing shared. Over time, repeated interactions might establish enough trust for deeper capabilities or information to become available. Access could remain narrow, purpose-specific, temporary, and revocable instead of turning into one giant yes/no permission boundary.

I don't know what that actually looks like yet. I only know that "connected" and "trusted" should not become synonyms, especially if personal AI systems eventually hold the most intimate parts of people's lives.

I would rather have them default to restraint than openness.

## Why local AI matters to me

A lot of this comes back to control.

AI may become one of the most important interfaces between people and computers. If that happens, I don't want the only serious versions of that interface to exist inside a handful of companies.

That does not mean large providers are automatically malicious. It means dependence is dependence.

If the model, memory, tools, permissions, data, and system intelligence all live somewhere else, then the user's relationship with their own computer starts depending on whoever owns that infrastructure.

Local AI gives us another path: not necessarily the most powerful model or the easiest system, but one where the machine can still belong to the person using it in a meaningful sense. Important state can stay under their control, behavior can be inspected and changed, inference engines can be replaced, and experiments in how AI interacts with the rest of the system do not require permission from a remote platform.

That possibility is enough reason for me to explore it.

## What `llm-app` is for now

All of this is much larger than the project in its current form.

I've only been working on this specific project for a short time. I don't know exactly what it wants to become yet, and I don't want to pretend otherwise.

Right now the goal is simpler: build a really clean local inference application and use it to discover the boundaries the larger system actually needs.

Loading models should be reliable, generation predictable, ownership explicit, and the interfaces responsive. The runtime should be reusable rather than trapped behind one frontend. Performance should be measured instead of assumed. Context and workflow infrastructure should be substantial enough to support real experiments, but nothing gets to survive merely because it was difficult to build.

I know that starting with a wide foundation sounds contradictory when one of the goals is avoiding bloat. For now, I think that is acceptable because I am mapping the space.

The important part is that the map does not become the territory. Once the system is usable, I want to cut ruthlessly. If an abstraction does not make the project clearer, safer, faster, or more capable, it should probably disappear.

The project should earn its complexity.

## Where I hope this leads

A very good local inference application with a clean Rust runtime would already be worthwhile.

From there, the more interesting possibilities can prove themselves instead of existing only as plans: context that behaves more like a navigable memory than a transcript; specialist models that prepare, verify, or react to work around a larger model; workflows that can reshape the path from input to output without depending on a hosted orchestration service; and a mesh that lets those pieces live across the machines a person already owns.

The runtime may become useful from a desktop, a terminal, a headless node, or something I have not thought of yet. I may find places where current inference engines are poorly suited to local hardware and replace pieces, or discover that the mature implementations were right and keep them. Some of these ideas may eventually flow back into the OS project.

The larger vision could also collapse under real-world constraints and leave me throwing most of it away. I am fine with that.

The point of this project is not to prove that I already know the answer. It is to build close enough to the problem that I can discover better questions.

The idea I keep coming back to is this:

> What would a local AI system look like if the model, context, tools, runtime, and computer were designed to work together from the beginning instead of being connected afterward through layers of historical glue?

I don't know.

That's why I'm building it.
