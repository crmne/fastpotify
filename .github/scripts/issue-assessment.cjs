// Shared by the assessment's preparation and its workflow_run completion hook.
const subjectQuery = `query($id: ID!) {
  node(id: $id) {
    id __typename
    ... on Reactable { reactionGroups { content viewerHasReacted } }
    ... on Issue { repository { nameWithOwner } }
    ... on Discussion { repository { nameWithOwner } }
    ... on IssueComment { updatedAt repository { nameWithOwner } }
    ... on DiscussionComment {
      updatedAt discussion { repository { nameWithOwner } }
    }
  }
}`;

async function readSubject(github, context, target) {
  if (typeof target?.subjectId !== "string" || !target.subjectId) {
    throw new Error("An assessment subject is required");
  }
  const { node } = await github.graphql(subjectQuery, { id: target.subjectId });
  if (!node) return null; // A deleted comment no longer needs a reaction.
  const repository = node.repository || node.discussion?.repository;
  if (!["Issue", "Discussion", "IssueComment", "DiscussionComment"].includes(node.__typename)
      || repository?.nameWithOwner !== `${context.repo.owner}/${context.repo.repo}`) {
    throw new Error("The assessment subject must belong to this repository");
  }
  if (node.__typename.endsWith("Comment")
      && (!target.updatedAt || Date.parse(node.updatedAt) !== Date.parse(target.updatedAt))) {
    return null; // An edit has its own assessment. Do not mark an older version.
  }
  return node;
}

async function prepare({ github, context }) {
  const { issue, discussion, comment, sender } = context.payload;
  const isBot = user => user?.type === "Bot" || user?.login?.endsWith("[bot]");
  if (issue?.pull_request || isBot(sender) || isBot(comment?.user)) return null;

  let subject = comment || issue || discussion;
  if (!subject && context.eventName === "workflow_dispatch") {
    const routed = JSON.parse(context.payload.inputs?.aw_context || "{}");
    const number = Number(routed.item_number);
    if (!["issue", "discussion"].includes(routed.item_type)
        || !Number.isSafeInteger(number) || number <= 0) {
      throw new Error("An issue or discussion number is required");
    }
    if (routed.item_type === "issue") {
      const { data } = await github.rest.issues.get({ ...context.repo, issue_number: number });
      if (data.pull_request) return null;
      subject = data;
    } else {
      const { repository } = await github.graphql(`query($owner: String!, $repo: String!, $number: Int!) {
        repository(owner: $owner, name: $repo) { discussion(number: $number) { id } }
      }`, { ...context.repo, number });
      subject = { node_id: repository.discussion?.id };
    }
  }
  const target = { subjectId: subject?.node_id, updatedAt: comment?.updated_at || null };
  const node = await readSubject(github, context, target);
  if (!node) return null;
  // Remove only this workflow token's reaction. A rocket never gates a retry.
  if (node.reactionGroups.some(group => group.content === "ROCKET" && group.viewerHasReacted)) {
    await github.graphql(`mutation($subjectId: ID!) {
      removeReaction(input: {subjectId: $subjectId, content: ROCKET}) { subject { id } }
    }`, { subjectId: target.subjectId });
  }
  return target;
}

async function complete({ github, context, target }) {
  const run = context.payload.workflow_run;
  if (run?.conclusion !== "success"
      || run.path !== ".github/workflows/issue-assessment.lock.yml"
      || run.head_branch !== context.payload.repository.default_branch
      || run.head_repository?.full_name !== `${context.repo.owner}/${context.repo.repo}`) {
    return;
  }
  if (!await readSubject(github, context, target)) return;
  await github.graphql(`mutation($subjectId: ID!) {
    addReaction(input: {subjectId: $subjectId, content: ROCKET}) { subject { id } }
  }`, { subjectId: target.subjectId });
}

function targetArtifactName(artifacts, attempt) {
  // Rerunning only failed jobs reuses preparation from an earlier attempt.
  return artifacts
    .filter(a => /^assessment-target-[1-9]\d*$/.test(a.name) && !a.expired)
    .map(a => ({ name: a.name, attempt: Number(a.name.slice('assessment-target-'.length)) }))
    .filter(a => a.attempt <= attempt)
    .sort((a, b) => b.attempt - a.attempt)[0]?.name || '';
}

module.exports = { prepare, complete, targetArtifactName };
