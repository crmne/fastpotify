const assert = require('node:assert/strict');
const { test } = require('node:test');
const { prepare, complete, targetArtifactName } = require('./issue-assessment.cjs');

function fixture(type = 'Issue') {
  const updatedAt = '2026-09-07T12:00:00Z';
  const node = {
    id: 'subject', __typename: type, updatedAt,
    repository: { nameWithOwner: 'crmne/fastpotify' },
    reactionGroups: [{ content: 'ROCKET', viewerHasReacted: false }],
  };
  const mutations = [];
  const context = {
    repo: { owner: 'crmne', repo: 'fastpotify' }, eventName: 'issues',
    payload: {
      issue: { number: 350, node_id: 'subject' }, sender: { type: 'User', login: 'reporter' },
      repository: { default_branch: 'main' },
      workflow_run: {
        conclusion: 'success', path: '.github/workflows/issue-assessment.lock.yml',
        head_branch: 'main', head_repository: { full_name: 'crmne/fastpotify' },
      },
    },
  };
  if (type === 'Discussion' || type === 'DiscussionComment') {
    delete context.payload.issue;
    context.payload.discussion = { number: 350, node_id: 'subject' };
    context.eventName = 'discussion';
  }
  if (type.endsWith('Comment')) {
    context.eventName = type === 'IssueComment' ? 'issue_comment' : 'discussion_comment';
    context.payload.comment = { node_id: 'subject', updated_at: updatedAt, user: { type: 'User' } };
  }
  if (type === 'DiscussionComment') {
    node.discussion = { repository: node.repository };
    delete node.repository;
  }
  const github = {
    graphql: async (query, variables) => {
      if (query.startsWith('mutation')) {
        mutations.push({ operation: query.includes('removeReaction') ? 'remove' : 'add', ...variables });
        return {};
      }
      return { node };
    },
  };
  return { github, context, node, mutations };
}

for (const type of ['Issue', 'Discussion', 'IssueComment', 'DiscussionComment']) {
  test(`${type}: successful assessment marks the triggering subject only after completion`, async () => {
    const f = fixture(type);
    const target = await prepare(f);
    assert.equal(target.subjectId, 'subject');
    assert.deepEqual(f.mutations, []);
    await complete({ ...f, target });
    assert.deepEqual(f.mutations, [{ operation: 'add', subjectId: 'subject' }]);
  });
}

test('an existing bot rocket is cleared and never blocks reassessment or retry', async () => {
  const f = fixture();
  f.node.reactionGroups[0].viewerHasReacted = true;
  const target = await prepare(f);
  assert.ok(target);
  assert.deepEqual(f.mutations, [{ operation: 'remove', subjectId: 'subject' }]);
  f.context.payload.workflow_run.conclusion = 'failure';
  await complete({ ...f, target });
  assert.equal(f.mutations.length, 1);
  assert.ok(await prepare(f));
});

test('maintainer and reporter reactions are preserved', async () => {
  const f = fixture();
  assert.ok(await prepare(f));
  assert.deepEqual(f.mutations, []);
});

for (const conclusion of ['failure', 'cancelled', 'timed_out', 'skipped', 'action_required']) {
  test(`${conclusion} does not get a completion marker`, async () => {
    const f = fixture();
    const target = await prepare(f);
    f.context.payload.workflow_run.conclusion = conclusion;
    await complete({ ...f, target });
    assert.deepEqual(f.mutations, []);
  });
}

for (const user of [{ type: 'Bot', login: 'github-actions' }, { login: 'github-actions[bot]' }]) {
  test(`bot comments cannot start a loop (${user.login})`, async () => {
    const f = fixture('IssueComment');
    f.context.payload.comment.user = user;
    assert.equal(await prepare(f), null);
    assert.deepEqual(f.mutations, []);
  });
}

test('bot edits and pull request comments are filtered', async () => {
  const f = fixture('IssueComment');
  f.context.payload.sender.type = 'Bot';
  assert.equal(await prepare(f), null);
  f.context.payload.sender.type = 'User';
  f.context.payload.issue.pull_request = {};
  assert.equal(await prepare(f), null);
  assert.deepEqual(f.mutations, []);
});

for (const type of ['IssueComment', 'DiscussionComment']) {
  test(`${type}: an edit during assessment cannot be marked by the older run`, async () => {
    const f = fixture(type);
    const target = await prepare(f);
    f.node.updatedAt = '2026-09-07T12:01:00Z';
    await complete({ ...f, target });
    assert.deepEqual(f.mutations, []);
    assert.equal(await prepare(f), null);
    f.context.payload.comment.updated_at = f.node.updatedAt;
    const edited = await prepare(f);
    await complete({ ...f, target: edited });
    assert.equal(f.mutations.length, 1);
  });
}

test('completion accepts only this workflow on the default branch of this repository', async () => {
  for (const change of [
    { path: '.github/workflows/other.yml' }, { head_branch: 'contributor' },
    { head_repository: { full_name: 'someone/fastpotify' } },
  ]) {
    const f = fixture();
    Object.assign(f.context.payload.workflow_run, change);
    await complete({ ...f, target: { subjectId: 'subject' } });
    assert.deepEqual(f.mutations, []);
  }
});

test('artifact data cannot direct a reaction outside the repository', async () => {
  const f = fixture();
  f.node.repository.nameWithOwner = 'someone/else';
  await assert.rejects(complete({ ...f, target: { subjectId: 'subject' } }), /this repository/);
  assert.deepEqual(f.mutations, []);
});

test('missing or deleted subjects cannot receive a completion marker', async () => {
  const f = fixture();
  await assert.rejects(prepare({ ...f, context: { ...f.context, payload: {} } }), /subject is required/);
  f.github.graphql = async () => ({ node: null });
  assert.equal(await prepare(f), null);
  await complete({ ...f, target: { subjectId: 'deleted' } });
  assert.deepEqual(f.mutations, []);
});

test('manual issue assessment resolves the issue and rejects pull requests', async () => {
  const f = fixture();
  f.context.eventName = 'workflow_dispatch';
  delete f.context.payload.issue;
  f.context.payload.inputs = { aw_context: JSON.stringify({ item_type: 'issue', item_number: 350 }) };
  const data = { node_id: 'subject' };
  f.github.rest = { issues: { get: async args => {
    assert.equal(args.issue_number, 350);
    return { data };
  } } };
  assert.ok(await prepare(f));
  data.pull_request = {};
  assert.equal(await prepare(f), null);
});

test('manual discussion assessment resolves the discussion', async () => {
  const f = fixture('Discussion');
  f.context.eventName = 'workflow_dispatch';
  delete f.context.payload.discussion;
  f.context.payload.inputs = { aw_context: JSON.stringify({ item_type: 'discussion', item_number: 350 }) };
  const graphql = f.github.graphql;
  f.github.graphql = async (query, args) => args.number
    ? { repository: { discussion: { id: 'subject' } } } : graphql(query, args);
  assert.ok(await prepare(f));
});

test('invalid manual context and API failures fail preparation instead of looking assessed', async () => {
  const f = fixture();
  f.context.eventName = 'workflow_dispatch';
  delete f.context.payload.issue;
  for (const aw_context of ['{', '{}', '{"item_type":"issue","item_number":-1}']) {
    f.context.payload.inputs = { aw_context };
    await assert.rejects(prepare(f));
  }
  const failing = fixture();
  failing.github.graphql = async () => { throw new Error('API unavailable'); };
  await assert.rejects(prepare(failing), /API unavailable/);
  assert.deepEqual(failing.mutations, []);
});

test('rerunning failed jobs can mark the assessment prepared by an earlier attempt', () => {
  const artifacts = [{ name: 'assessment-target-1', expired: false }];
  assert.equal(targetArtifactName(artifacts, 2), 'assessment-target-1');
  artifacts.push({ name: 'assessment-target-2', expired: false });
  assert.equal(targetArtifactName(artifacts, 2), 'assessment-target-2');
  artifacts.push({ name: 'assessment-target-10', expired: false });
  assert.equal(targetArtifactName(artifacts, 10), 'assessment-target-10');
  assert.equal(targetArtifactName(artifacts, 1), 'assessment-target-1');
});

test('filtered events, expired artifacts, and unrelated artifacts have no completion target', () => {
  assert.equal(targetArtifactName([], 1), '');
  assert.equal(targetArtifactName([
    { name: 'assessment-target-1', expired: true },
    { name: 'agent', expired: false },
    { name: 'assessment-target-2', expired: false },
  ], 1), '');
});
