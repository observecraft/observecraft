import assert from 'node:assert/strict';
import test from 'node:test';
import { catalogEntry, classifierTags, createTemplates, finalizeCatalog, resolveSpec } from './import-integration-catalog.mjs';
import { hasReportedMetrics } from '../web/src/api/integrations/detection.ts';
import { instanceOptions, renderAgentConfig } from '../web/src/api/integrations/agentConfig.ts';

const TEMPLATES = {
  instances: { name: 'instances', multiple: true, description: 'Every instance is scheduled independently.' },
  init_config: { name: 'init_config', description: 'Options for all instances.' },
  'instances/default': [{ template: 'instances/tags' }, { name: 'min_collection_interval', description: 'Seconds between runs.', value: { type: 'number', example: 15 } }],
  'instances/tags': { name: 'tags', description: 'Tags for every metric.', value: { type: 'array', items: { type: 'string' }, example: ['<KEY_1>:<VALUE_1>'] } },
  'init_config/default': [{ name: 'service', description: 'Service tag.', value: { type: 'string' } }],
  'instances/http': [
    { name: 'username', description: 'User.', value: { type: 'string' } },
    { name: 'password', description: 'Password.', secret: true, value: { type: 'string' } },
    { name: 'timeout', description: 'Timeout.', value: { type: 'number', example: 10 } },
  ],
};
const templates = createTemplates((path) => TEMPLATES[path]);

test('classifier tags split into categories, operating systems, offerings and data types', () => {
  assert.deepEqual(
    classifierTags(['Category::Caching', 'Category::Data Stores', 'Supported OS::Linux', 'Supported OS::macOS', 'Offering::Integration', 'Submitted Data Type::Metrics', 'Queried Data Type::Logs']),
    { categories: ['Caching', 'Data Stores'], supportedOs: ['Linux', 'macOS'], offerings: ['Integration'], dataTypes: ['Metrics', 'Logs'] },
  );
});

test('templates referenced by a section are loaded and its own fields win', () => {
  const spec = resolveSpec(
    {
      name: 'Redis',
      files: [{ name: 'redisdb.yaml', options: [{ template: 'instances', options: [{ name: 'host', required: true, description: 'Host.', value: { type: 'string', example: 'localhost' } }] }] }],
    },
    templates,
  );
  const [instances] = spec.files[0].options;
  assert.equal(instances.name, 'instances');
  assert.equal(instances.multiple, true);
  assert.deepEqual(
    instances.options.map((o) => o.name),
    ['host'],
  );
});

test('list templates are spliced in place, nested ones too, preserving order', () => {
  const spec = resolveSpec(
    { files: [{ name: 'x.yaml', options: [{ template: 'instances', options: [{ name: 'host', description: 'Host.', value: { type: 'string' } }, { template: 'instances/default' }] }] }] },
    templates,
  );
  assert.deepEqual(
    spec.files[0].options[0].options.map((o) => o.name),
    ['host', 'tags', 'min_collection_interval'],
  );
});

test('overrides change a named option of a template', () => {
  const spec = resolveSpec(
    { files: [{ name: 'x.yaml', options: [{ template: 'instances', options: [{ template: 'instances/http', overrides: { 'timeout.value.example': 30, 'username.required': true } }] }] }] },
    templates,
  );
  const options = spec.files[0].options[0].options;
  assert.equal(options.find((o) => o.name === 'timeout').value.example, 30);
  assert.equal(options.find((o) => o.name === 'username').required, true);
});

test('a hidden template hides the options it brings and those after it until the next template', () => {
  const spec = resolveSpec(
    {
      files: [
        {
          name: 'x.yaml',
          options: [
            { template: 'instances', options: [{ template: 'instances/http', hidden: true }, { name: 'legacy', description: 'Old.', value: { type: 'string' } }, { template: 'instances/tags' }] },
          ],
        },
      ],
    },
    templates,
  );
  const hidden = Object.fromEntries(spec.files[0].options[0].options.map((o) => [o.name, o.hidden]));
  assert.deepEqual(hidden, { username: true, password: true, timeout: true, legacy: true, tags: false });
});

test('a missing template fails with its name', () => {
  assert.throws(() => resolveSpec({ files: [{ name: 'x.yaml', options: [{ template: 'instances/nope' }] }] }, templates), /instances\/nope/);
});

const REDIS_MANIFEST = {
  app_id: 'redis',
  tile: {
    title: 'Redis',
    description: 'Track redis performance.',
    classifier_tags: ['Category::Caching', 'Supported OS::Linux', 'Offering::Integration'],
  },
  author: { name: 'Datadog' },
  assets: {
    integration: { metrics: { prefix: 'redis.', check: 'redis.net.clients' }, process_signatures: ['redis-server'], configuration: { spec: 'assets/configuration/spec.yaml' } },
    dashboards: { redis: 'assets/dashboards/overview.json' },
    monitors: { 'Memory consumption is high': 'assets/monitors/high_mem.json' },
    logs: { source: 'redis' },
  },
};

test('a manifest becomes a catalog entry', () => {
  assert.deepEqual(catalogEntry('redisdb', REDIS_MANIFEST), {
    id: 'redis',
    check: 'redisdb',
    name: 'Redis',
    publisher: 'Datadog',
    description: 'Track redis performance.',
    categories: ['Caching'],
    supportedOs: ['Linux'],
    offerings: ['Integration'],
    dataTypes: [],
    metricPrefix: 'redis.',
    checkMetrics: ['redis.net.clients'],
    processSignatures: ['redis-server'],
    logSource: 'redis',
    dashboards: ['redis'],
    monitors: ['Memory consumption is high'],
    configurable: true,
  });
});

test('integrations without metrics or configuration have empty fields, not missing ones', () => {
  const entry = catalogEntry('azure_active_directory', {
    app_id: 'azure-active-directory',
    tile: { title: 'Microsoft Entra ID', description: 'Logs.', classifier_tags: ['Category::Azure'] },
    author: { name: 'Datadog' },
    assets: { integration: { configuration: {} } },
  });
  assert.equal(entry.metricPrefix, null);
  assert.deepEqual(entry.checkMetrics, []);
  assert.equal(entry.configurable, false);
  assert.equal(entry.logSource, null);
});

test('the catalog prefix detects an integration whose name differs from its metrics', () => {
  assert.equal(hasReportedMetrics('redis', [], new Set(['redis.net.clients']), ['redis.']), true);
  assert.equal(hasReportedMetrics('ibm_mq', [], new Set(['ibm_mq.queue.depth_current']), ['ibm_mq.']), true);
  assert.equal(hasReportedMetrics('ibm_mq', [], new Set(['redis.net.clients']), ['ibm_mq.']), false);
});

const REDIS_FILE = {
  name: 'redisdb.yaml',
  options: [
    { name: 'init_config', options: [{ name: 'service', value: { type: 'string' } }] },
    {
      name: 'instances',
      multiple: true,
      options: [
        { name: 'host', required: true, value: { type: 'string' } },
        { name: 'port', required: true, value: { type: 'integer' } },
        { name: 'password', secret: true, value: { type: 'string' } },
        { name: 'tags', value: { type: 'array', items: { type: 'string' } } },
        { name: 'ssl', value: { type: 'boolean' } },
      ],
    },
  ],
};

test('the Agent configuration has the filled options with their types', () => {
  const yaml = renderAgentConfig(REDIS_FILE, { host: 'localhost', port: '6379', password: '', tags: 'env:prod, team:storage', ssl: 'true' });
  assert.equal(yaml, ['init_config: {}', '', 'instances:', '  - host: localhost', '    port: 6379', '    tags:', '      - env:prod', '      - team:storage', '    ssl: true', ''].join('\n'));
});

test('values that look like other YAML types stay strings', () => {
  const yaml = renderAgentConfig(REDIS_FILE, { host: 'yes', port: '6379' });
  assert.match(yaml, /host: 'yes'/);
});

test('options typed with anyOf stay in the form, as their first scalar or string-list form', () => {
  const file = {
    name: 'kafka_consumer.yaml',
    options: [{ name: 'instances', options: [{ name: 'kafka_connect_str', required: true, value: { anyOf: [{ type: 'string' }, { type: 'array', items: { type: 'string' } }] } }] }],
  };
  assert.deepEqual(
    instanceOptions(file).map((o) => o.name),
    ['kafka_connect_str'],
  );
  assert.match(renderAgentConfig(file, { kafka_connect_str: 'broker-1:9092' }), /- kafka_connect_str: broker-1:9092/);
});

test('integrations sharing an app id get their check name as id; shared or generic prefixes do not detect', () => {
  const entry = (id, check, metricPrefix) => ({ id, check, name: check, metricPrefix });
  const catalog = finalizeCatalog([
    entry('system', 'disk', 'system.'),
    entry('system', 'system_core', 'system.'),
    entry('kafka', 'kafka', 'kafka.'),
    entry('kafka-consumer', 'kafka_consumer', 'kafka.'),
    entry('redis', 'redisdb', 'redis.'),
    entry('container', 'container', 'container.'),
  ]);
  assert.deepEqual(
    catalog.map((e) => [e.id, e.detectionPrefix]),
    [
      ['disk', null],
      ['system-core', null],
      ['kafka', null],
      ['kafka-consumer', null],
      ['redis', 'redis.'],
      ['container', null],
    ],
  );
});

test('a check name that collides with another app id falls back to check names for both', () => {
  const catalog = finalizeCatalog([
    { id: 'system', check: 'network', metricPrefix: null },
    { id: 'system', check: 'disk', metricPrefix: null },
    { id: 'network', check: 'http_check', metricPrefix: null },
  ]);
  assert.deepEqual(
    catalog.map((e) => e.id),
    ['network', 'disk', 'http-check'],
  );
});

test('curated entries are linked to their integrations-core entry; the others stay catalog-only', async () => {
  const { linkCore } = await import('../web/src/api/integrations/coreLink.ts');
  const core = [
    { id: 'redis', check: 'redisdb', name: 'Redis' },
    { id: 'postgres', check: 'postgres', name: 'Postgres' },
    { id: 'kubernetes-cluster-autoscaler', check: 'kubernetes_cluster_autoscaler', name: 'Kubernetes Cluster Autoscaler' },
    { id: 'hugging-face-tgi', check: 'hugging_face_tgi', name: 'Hugging Face TGI' },
    { id: 'ibm-mq', check: 'ibm_mq', name: 'IBM MQ' },
  ];
  const curated = [
    { id: 'redis', name: 'Redis' },
    { id: 'postgresql', name: 'PostgreSQL' },
    { id: 'cluster-autoscaler', name: 'Kubernetes Cluster Autoscaler' },
    { id: 'huggingface', name: 'Hugging Face TGI' },
    { id: 'slack', name: 'Slack' },
  ];
  const { linked, unlinked } = linkCore(curated, core);
  assert.deepEqual(Object.fromEntries([...linked].map(([id, c]) => [id, c.check])), {
    redis: 'redisdb',
    postgresql: 'postgres',
    'cluster-autoscaler': 'kubernetes_cluster_autoscaler',
    huggingface: 'hugging_face_tgi',
  });
  assert.deepEqual(
    unlinked.map((c) => c.id),
    ['ibm-mq'],
  );
});
