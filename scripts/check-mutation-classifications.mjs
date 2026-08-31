#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const [outputDirectory, classificationPath] = process.argv.slice(2);
if (!outputDirectory || !classificationPath) {
  console.error("usage: check-mutation-classifications.mjs MUTANTS_OUT CLASSIFICATIONS_JSON");
  process.exit(2);
}

const lines = (name) => {
  const target = path.join(outputDirectory, name);
  if (!fs.existsSync(target)) return [];
  return fs
    .readFileSync(target, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
};

const timeouts = lines("timeout.txt");
if (timeouts.length > 0) {
  console.error(`mutation lane has ${timeouts.length} timeout(s); classifications cannot hide timeouts`);
  process.exit(1);
}

const missed = lines("missed.txt");
if (missed.length === 0) {
  console.error("cargo-mutants failed without a classifiable missed mutant");
  process.exit(1);
}

const document = JSON.parse(fs.readFileSync(classificationPath, "utf8"));
const accepted = new Map(document.classifications.map((entry) => [entry.mutant, entry]));
const unexplained = missed.filter((mutant) => !accepted.has(mutant));
if (unexplained.length > 0) {
  console.error("unclassified surviving mutants:");
  for (const mutant of unexplained) console.error(`- ${mutant}`);
  process.exit(1);
}

for (const mutant of missed) {
  const entry = accepted.get(mutant);
  if (!document.policy.allowed_classifications.includes(entry.classification)) {
    console.error(`invalid classification '${entry.classification}' for ${mutant}`);
    process.exit(1);
  }
  if (typeof entry.explanation !== "string" || entry.explanation.trim().length < 20) {
    console.error(`classification explanation is missing or too short for ${mutant}`);
    process.exit(1);
  }
  console.log(`CLASSIFIED ${entry.classification}: ${mutant}`);
}

const report = {
  schema_version: 1,
  cargo_mutants_version: document.cargo_mutants_version,
  missed: missed.map((mutant) => accepted.get(mutant)),
  timeouts,
};
fs.writeFileSync(
  path.join(outputDirectory, "classification-report.json"),
  `${JSON.stringify(report, null, 2)}\n`,
);
