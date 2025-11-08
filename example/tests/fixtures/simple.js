console.log("[bootstrap] greeting:", greeting);
console.log("[bootstrap] host.info:", JSON.stringify(host.info));

function describeHost(source) {
	return `${source} is talking to ${host.info.name} v${host.info.version}`;
}

function jsMultiply(lhs, rhs) {
	return Number(lhs) * Number(rhs) * host.info.multiplier;
}

const echoed = host.echo("[bootstrap] host.echo invoked");
console.log("[bootstrap] host.echo returned:", echoed);

"bootstrap-complete";
