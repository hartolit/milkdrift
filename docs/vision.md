# Vision

I started this project because I think the way we currently connect AI to computers is still strangely primitive.

We have models capable of reasoning across huge amounts of information, yet when we want them to actually *do* something, we often reduce the interaction to a giant pile of text: JSON schemas, HTTP APIs, JavaScript wrappers, tool descriptions, serialized state, verbose responses, and increasingly high-level actions that have to be explained to the model before it can use them.

It works, but I don't think it is where this ends.

The model's context is one of the most valuable resources in the entire system. Every tool description, protocol field, repeated state object, irrelevant search result, and implementation detail competes with the actual problem the model is trying to reason about. I want to treat that context as something almost sacred: keep it clean, dense, and focused on the information that matters.

That is the idea underneath this project.

Not just another local LLM interface, and not an attempt to reimplement everything that Candle, llama.cpp, or other inference engines already do well. I want to build the foundations for a local AI system where inference, tools, context, and the computer itself can eventually fit together much more naturally.

For now, it starts as an inference app.

That is intentional.

## The part that bothers me

A lot of modern AI tooling feels like we are bolting intelligence onto software that was designed for something else.

The model sits at the top, and underneath it is layer after layer of abstractions made for humans, web applications, distributed services, or conventional programs. To search the web, inspect a file, query a database, compile code, or interact with the operating system, the model often needs a textual description of some API, then has to produce another textual representation of a call, which gets translated through several layers before anything actually happens.

Then the result comes back as more text.

Sometimes this is exactly the right design. HTTP and JSON are popular for good reasons. Standard interfaces make systems interoperable, inspectable, and easy to build around.

But I don't think they should automatically become the language between an AI and the machine it lives on.

If a model is running locally, why should every internal capability pretend to be a web service?

Why should a tool with a small, well-defined operation need a giant schema injected into context?

Why should the primary reasoning model spend tokens reading through fifty search results when a smaller, faster model could inspect them first and return the three pieces that actually matter?

Why should the model repeatedly reconstruct system state from text when the runtime already knows that state exactly?

Those questions are more interesting to me than building another chat window.

## Keep the context clean

The clearest example is web search.

If the main model asks a focused question, I don't necessarily want it to become the search engine too. I would rather hand that question to a smaller, fast agent whose entire job is to search, inspect, discard noise, preserve useful sources, and return a compact result.

The expensive model gets the information it needs.

The search agent gets the messy retrieval problem.

The context window does not become a dumping ground for the internet.

That same idea could apply everywhere.

A code-search agent might understand an indexed repository without feeding the primary model thousands of unrelated lines. A compiler integration could reduce pages of diagnostics into the exact failures relevant to the current change while keeping the raw evidence outside the main context. A filesystem tool could expose a narrow capability without teaching the model an entire shell environment. A system service could keep its internal state in memory and reveal only the parts needed for the current decision.

I don't know yet what the right abstraction is.

It might involve typed handles, capability-based APIs, compact binary representations, specialist models, deterministic processors, or something I haven't encountered yet.

The important part is the direction: **the model should reason about meaning, not spend half of its attention translating plumbing.**

## A local AI system, not just a model runner

This is where the project starts becoming more ambitious.

I don't really want to build an application where the UI is the product and the inference engine is the interesting part underneath.

I want the system around the model to become interesting too.

The inference backend should eventually be replaceable. Candle might be right for one path, llama.cpp for another, and something else might make more sense later. The project should own the behavior that matters above those implementations: lifecycle, context, memory, tools, permissions, state, orchestration, and the way the AI experiences the rest of the machine.

That also gives me room to experiment with inference itself.

I want to understand what current inference stacks are doing, where they are wasting time or memory on local machines, and which parts can be shaped differently when the entire system is designed around one local AI rather than a general-purpose framework.

I may discover that most of the underlying work is already better solved elsewhere.

That would be useful too.

The point is not to replace mature libraries for the sake of owning more code. The point is to get close enough to the machine that I can tell which layers are essential and which ones only exist because that is how software has traditionally been assembled.

## The larger idea

I have another project sitting on the side: a custom x86_64 operating system built around an ECS-like architecture.

The original thought was simple: if an AI were a first-class participant in an operating system rather than an application bolted onto one, what would the machine look like?

Would it still expose itself through the same collection of files, processes, command lines, configuration formats, services, and protocols?

Or could the system present a much cleaner model of what exists, what owns what, what can be changed, and what each capability is allowed to do?

An ECS-style system is interesting to me because it can make the machine look less like a pile of opaque programs and more like a world of explicit state and capabilities. That feels naturally compatible with an AI that needs to understand and manipulate a system without being given unrestricted access to everything.

I don't know if that idea is good yet.

I've put the OS aside because I don't want to disappear into years of low-level implementation before understanding whether the premise is even useful. This inference project is a much better place to learn. It can become a real application, expose the actual pain points, and still leave me with something worthwhile if the larger idea turns out to be wrong.

That matters to me. I don't want the grand vision to become an excuse to build nothing.

## Uniqueness, standardization, and security

One of the stranger ideas behind the OS project is a suspicion that extreme standardization may become a bigger security problem as automated attackers become more capable.

Today, millions of machines often run the same operating systems, libraries, services, protocols, and deployment patterns. That standardization is incredibly useful, but it also means one successful exploit can sometimes be turned into a repeatable recipe.

Now imagine attackers that can reason, adapt, probe, generate exploits, and operate continuously at machine speed.

The attacker is no longer one person patiently studying one target. It can become a huge population of automated systems constantly testing everything they can reach.

In that world, I wonder whether some amount of system-level uniqueness becomes valuable.

Not "security through obscurity" in the old sense. Hiding a broken system does not make it secure. A unique buffer overflow is still a buffer overflow.

What interests me is the possibility of systems that share strong security principles but do not all share the exact same internal structure.

Maybe two machines can agree on how to communicate without having identical internals. Maybe capabilities can be composed differently. Maybe layouts, services, execution paths, and internal relationships can be inferred or generated for the environment rather than copied from one universal distribution.

Then compromising one machine does not automatically reveal a perfect map of the next million.

I have no proof that this would work. It could easily create new problems: harder auditing, harder updates, harder reproducibility, and a much larger space for subtle mistakes.

But I think the question is worth asking.

The future probably needs both: common ground where communication and verification matter, and diversity where identical internal structure only makes large-scale exploitation easier.

## Trust should probably feel more human

The same uncertainty applies to communication between AI systems.

I don't like the idea that authentication automatically means trust.

People don't usually meet someone and immediately hand over their private history, personal thoughts, credentials, and authority to act on their behalf. Trust develops slowly. It is contextual. You might trust someone with one thing and not another. You can know someone for years and still keep boundaries.

A personal AI holding a large amount of information about its user may need something closer to that.

Two systems could begin with almost nothing shared. Over time, repeated interactions might establish enough trust for deeper capabilities or information to become available. Access could remain narrow, purpose-specific, temporary, and revocable instead of turning into one giant yes/no permission boundary.

Again, I don't know what that actually looks like yet.

I only know that "connected" and "trusted" should probably not become synonyms.

And if personal AI systems eventually hold the most intimate parts of people's lives, I would rather have them default to restraint than openness.

## Why local AI matters to me

A lot of this comes back to control.

AI may become one of the most important interfaces between people and computers. If that happens, I don't want the only serious versions of that interface to exist inside a handful of companies.

That does not mean large providers are automatically malicious. It means dependence is dependence.

If the model, memory, tools, permissions, data, and system intelligence all live somewhere else, then the user's relationship with their own computer starts depending on whoever owns that infrastructure.

Local AI gives us another path.

Not necessarily the most powerful model.

Not necessarily the easiest system.

But one where the machine can still belong to the person using it in a meaningful sense.

Where the important state can stay local.

Where the underlying behavior can be inspected and changed.

Where an application can choose different inference engines instead of becoming inseparable from one vendor.

Where experimentation with how AI interacts with the rest of the system is still possible.

That possibility is enough reason for me to explore it.

## What `llm-app` is for now

All of this is much larger than the project in its current form.

I've only been working on this specific project for a couple of days. I don't know exactly what it wants to become yet, and I don't want to pretend otherwise.

Right now the goal is much simpler:

Build a really clean local inference application.

Make loading models reliable. Make generation predictable. Keep the UI fast. Keep ownership explicit. Avoid unnecessary dependencies and layers. Make the runtime reusable. Understand the performance characteristics instead of hand-waving about them. Build enough context and workflow infrastructure to experiment, but be willing to delete anything that turns out to be pointless.

I know that building a wide foundation seems contradictory when one of the goals is avoiding bloat.

For now, I think that is acceptable.

I'm mapping the space.

The important part is that the wide base does not become permanent just because it exists. Once the system is usable, I want to cut ruthlessly. If an abstraction does not make the system clearer, safer, faster, or more capable, it should probably disappear.

The project should earn its complexity.

## Where I hope this leads

Maybe this ends as a very good local inference application with a clean Rust runtime.

That would already be worthwhile.

Maybe the context system becomes more important and leads to a different way of integrating tools.

Maybe small specialist models become part of the architecture and allow the main model to work with much cleaner information.

Maybe I find places where current inference engines are poorly suited to local hardware and start replacing pieces.

Maybe the runtime becomes useful outside the desktop application.

Maybe some of the ideas eventually flow back into the OS project.

Or maybe the larger vision collapses under real-world constraints and I throw most of it away.

I am fine with that.

The point of this project is not to prove that I already know the answer.

It is to build close enough to the problem that I can discover better questions.

The idea I keep coming back to is this:

> What would a local AI system look like if the model, context, tools, runtime, and computer were designed to work together from the beginning instead of being connected afterward through layers of historical glue?

I don't know.

That's why I'm building it.
