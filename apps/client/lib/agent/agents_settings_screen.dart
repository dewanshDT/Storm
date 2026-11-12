import 'agent_models.dart';

/// Why a host cannot install an agent, in words (decision 133).
String installBlockedText(String? code, String agent) => switch (code) {
  'no_node' => 'Needs Node.js on this host',
  'node_too_old' =>
    agent == 'command-code'
        ? 'Needs a newer Node.js on this host (22 or later)'
        : 'Needs a newer Node.js on this host',
  'unsupported_platform' => "Not available on this host's system yet",
  'disabled' => 'Installs are off on this host',
  _ => "Can't be installed on this host",
};

/// One line for a failed install, update or removal, from the job's or the
/// refusal's stable code. An unknown code is shown as it is.
String describeAgentJobError(String? code, {String agent = ''}) =>
    switch (code) {
      'download_failed' =>
        "The download failed, so nothing was installed. Check the host's "
            'network and try again.',
      'checksum_mismatch' =>
        "The download didn't match its checksum, so nothing was installed.",
      'unpack_failed' =>
        "The download couldn't be unpacked, so nothing was installed.",
      'npm_failed' => "npm couldn't install it on this host.",
      'version_probe_failed' =>
        "It installed, but didn't answer when asked its version.",
      'disk_full' => 'The host is out of disk space.',
      'not_managed' =>
        "Storm didn't install this copy, so it can't change it. Update or "
            'remove it the way you installed it.',
      'in_use' ||
      'agent_in_use' => 'Sessions are using it. End them and try again.',
      'busy' => 'The host is already changing an agent. Try again after.',
      'timeout' => 'It took too long, so the host stopped.',
      'no_node' ||
      'node_too_old' ||
      'unsupported_platform' ||
      'disabled' => '${installBlockedText(code, agent)}.',
      'unknown_agent' =>
        "This host doesn't know that agent. Update storm-runtime on it.",
      'host_too_old' => 'Update storm-runtime to manage agents here.',
      'host_offline' => 'The host is offline.',
      null || 'unknown' => 'Something went wrong on the host.',
      _ => 'Something went wrong on the host ($code).',
    };

/// An install, update or removal in flight on one agent on one host, or the
/// last one's failure.
class AgentRun {
  const AgentRun({required this.op, this.step, this.error});

  final String op;
  final String? step;
  final String? error;

  bool get active => error == null;

  /// Which of the host's three steps it is on, 0 before the first.
  int get stepNumber => switch (step) {
    'Downloading' => 1,
    'Verifying' => 2,
    'Installing' => 3,
    _ => 0,
  };

  String get progress => switch ((op, step)) {
    ('remove', _) => 'Removing…',
    (_, 'Downloading') => 'Downloading…',
    (_, 'Verifying') => 'Verifying…',
    (_, 'Installing') => 'Installing…',
    _ => 'Starting…',
  };
}

/// What one grid cell says (153-7): the agent on the host in one word.
enum CellWord { ready, signIn, update, install, busy, cantInstall, offline }

class AgentCell {
  const AgentCell(this.word, this.top, {this.sub = '', this.progress});

  final CellWord word;

  /// The word itself: "Ready", "Install", "Can't install", "—".
  final String top;

  /// The version, the step or the reason, in mono under it.
  final String sub;

  /// 0 to 1 while installing, updating or removing.
  final double? progress;

  /// Only a word you can act on is in `accent`.
  bool get actionable =>
      word == CellWord.signIn ||
      word == CellWord.update ||
      word == CellWord.install;
}

/// The cap a host reports for [agent], or null.
ProviderCap? capFor(AgentHost host, String agent) =>
    host.providers.where((p) => p.id == agent).firstOrNull;

bool isInstalled(ProviderCap p) => p.available || p.source != null;

/// A managed install with a newer version the host can fetch.
bool hasUpdate(ProviderCap p) =>
    p.managed && p.latest != null && p.version != null && p.latest != p.version;

/// Why it can't be installed, in the cell's few words.
String installBlockedShort(String? code, String agent) => switch (code) {
  'no_node' => 'needs Node.js',
  'node_too_old' =>
    agent == 'command-code' ? 'needs Node 22' : 'needs newer Node',
  'unsupported_platform' => 'not on this system',
  'disabled' => 'installs are off',
  _ => 'not available',
};

/// The cell for [agent] on [host], from what the host reports (decision 133)
/// and any run in flight.
AgentCell agentCell(AgentHost host, String agent, AgentRun? run) {
  if (!host.online) return const AgentCell(CellWord.offline, '—');
  if (run != null && run.active) {
    final word = switch (run.op) {
      'remove' => 'Removing',
      'update' => 'Updating',
      _ => 'Installing',
    };
    final n = run.stepNumber;
    return AgentCell(
      CellWord.busy,
      word,
      sub: run.op == 'remove' || n == 0 ? 'starting' : 'step $n of 3',
      progress: run.op == 'remove' ? 0.5 : (n == 0 ? 0.1 : n / 3),
    );
  }
  final p = capFor(host, agent);
  if (p == null) {
    return AgentCell(
      CellWord.cantInstall,
      "Can't install",
      sub: host.capabilitiesKnown ? 'update storm-runtime' : '',
    );
  }
  if (!isInstalled(p)) {
    if (!host.agentInstall) {
      return const AgentCell(
        CellWord.cantInstall,
        "Can't install",
        sub: 'update storm-runtime',
      );
    }
    if (!p.installable) {
      return AgentCell(
        CellWord.cantInstall,
        "Can't install",
        sub: installBlockedShort(p.installBlocked, agent),
      );
    }
    return const AgentCell(CellWord.install, 'Install');
  }
  if (host.agentInstall && hasUpdate(p)) {
    return AgentCell(
      CellWord.update,
      'Update',
      sub: '${p.version} → ${p.latest}',
    );
  }
  if (p.signedIn == false) {
    return AgentCell(CellWord.signIn, 'Sign in', sub: p.version ?? '');
  }
  return AgentCell(CellWord.ready, 'Ready', sub: p.version ?? '');
}
