import ForceGraph3D from "react-force-graph-3d";
import * as THREE from "three";

type Node = {
	id: string;
	url?: string;
	icon?: string;
	color?: string;
	desc?: string;
	opacity?: number;
	x?: number;
	y?: number;
	z?: number;
	vx?: number;
	vy?: number;
	vz?: number;
	fx?: number;
	fy?: number;
	fz?: number;
};
type Link = { source?: string; target?: string; value?: number };
type GraphData = {
	nodes: Node[];
	links: Link[];
};

const nodes = [
	{
		id: "Niclas Overby",
		desc: "Niclas Overby Ⓝ",
		icon: "me.avif",
	},
	{
		id: "Commerce",
		desc: "Commerce",
		icon: "commerce.avif",
		color: "#45b1e8",
	},
	{
		id: "Improve",
		desc: "Improve",
		icon: "improve.avif",
		color: "#7fff00",
	},
	{
		id: "Connect",
		desc: "Connect",
		icon: "connect.avif",
		color: "#e34234",
	},
	{
		id: "Immerse",
		desc: "Immerse",
		color: "#ff7f50",
		icon: "immerse.avif",
	},
	{
		id: "Give",
		desc: "Give",
		icon: "give.avif",
		color: "#6a5acd",
	},
	{
		id: "Fediverse",
		desc: "Fediverse\nInfo",
		icon: "fediverse.avif",
		color: "#000000",
		url: "https://fediverse.info",
	},
	{
		id: "LinkedIn",
		desc: "LinkedIn\nProfile",
		icon: "linkedin.avif",
		url: "https://www.linkedin.com/in/niclasoverby",
	},
	{
		id: "PixelFed",
		desc: "PixelFed\nProfile",
		icon: "pixelfed.avif",
		url: "https://pixelfed.social/niclasoverby",
	},
	{
		id: "Mail",
		desc: "Send Mail",
		icon: "mail.avif",
		url: "mailto:niclas@overby.me",
	},
	{
		id: "Matrix",
		desc: "Matrix\nProfile",
		icon: "matrix.avif",
		url: "https://matrix.to/#/@niclas:overby.me",
	},
	{
		id: "Signal",
		desc: "Signal\nProfile",
		icon: "signal.avif",
		url: "https://signal.me/#eu/BKjgrHvQhqgDPpy9p2VfcfVj6yx0mJtVGOX8GQ_2htxhX7cDxhREVad8oWL1qAMj",
	},
	{
		id: "Rocksky",
		desc: "Rocksky\nProfile",
		icon: "rocksky.avif",
		url: "https://rocksky.app/profile/overby.me",
	},
	{
		id: "Atmosphere",
		desc: "Atmosphere",
		icon: "atmosphere.avif",
		color: "#00ffff",
		opacity: 0.1,
		url: "https://atproto.com/",
	},
	{
		id: "Bridgy",
		desc: "Bridgy Fed",
		icon: "bridgy.avif",
		color: "#ffffff",
		opacity: 0.1,
		url: "https://fed.brid.gy",
	},
	{
		id: "GitHub",
		desc: "GitHub\nProfile",
		icon: "github.avif",
		url: "https://github.com/overby-me",
	},
	{
		id: "Codeberg",
		desc: "Codeberg\nProfile",
		icon: "codeberg.avif",
		url: "https://codeberg.org/overby-me",
	},
	{
		id: "Tangled",
		desc: "Tangled\nProfile",
		icon: "tangled.avif",
		url: "https://tangled.org/@overby.me",
	},
	{
		id: "Mastodon",
		desc: "Mastodon\nProfile",
		icon: "mastodon.avif",
		url: "https://mas.to/@niclasoverby",
	},
	{
		id: "Bluesky",
		desc: "Bluesky\nProfile",
		icon: "bluesky.avif",
		url: "https://bsky.app/profile/overby.me",
	},
	{
		id: "Radikale Venstre",
		desc: "Radikale Venstre\n(Political Effort)",
		icon: "radikale.avif",
		url: "https://www.radikale.dk",
	},
	{
		id: "Aivero",
		desc: "Aivero\n(Ex-company)",
		icon: "aivero.avif",
		url: "https://www.aivero.com",
	},
	{
		id: "Factbird",
		desc: "Factbird\n(Ex-company)",
		icon: "factbird.avif",
		url: "https://www.factbird.com",
	},
	{
		id: "Veo",
		desc: "Veo\n(Commercial Effort)",
		icon: "veo.avif",
		url: "https://www.veo.co",
	},
	{
		id: "Wikipedia",
		desc: "Wikipedia\nProfile",
		icon: "wikipedia.avif",
		url: "https://en.wikipedia.org/wiki/User:Niclas_Overby",
	},
	{
		id: "HappyCow",
		desc: "HappyCow\nProfile",
		icon: "happycow.avif",
		url: "https://www.happycow.net/members/profile/niclasoverby",
	},
	{
		id: "Lemmy",
		desc: "Lemmy\nProfile",
		icon: "lemmy.avif",
		url: "https://lemmy.world/u/noverby",
	},
	{
		id: "NeoDB",
		desc: "NeoDB\nProfile",
		icon: "neodb.avif",
		url: "https://neodb.social/users/niclasoverby",
	},
];

const links = [
	{ source: "Niclas Overby", target: "Commerce" },
	{ source: "Niclas Overby", target: "Improve" },
	{ source: "Niclas Overby", target: "Connect" },
	{ source: "Niclas Overby", target: "Immerse" },
	{ source: "Niclas Overby", target: "Give" },
	{ source: "Connect", target: "Mail" },
	{ source: "Connect", target: "Matrix" },
	{ source: "Connect", target: "LinkedIn" },
	{ source: "Connect", target: "Mastodon" },
	{ source: "Connect", target: "PixelFed" },
	{ source: "Connect", target: "Bluesky" },
	{ source: "Connect", target: "Signal" },
	{ source: "Commerce", target: "LinkedIn" },
	{ source: "Commerce", target: "Aivero" },
	{ source: "Commerce", target: "Factbird" },
	{ source: "Commerce", target: "Veo" },
	{ source: "Commerce", target: "GitHub" },
	{ source: "Immerse", target: "PixelFed" },
	{ source: "Immerse", target: "Rocksky" },
	{ source: "Immerse", target: "NeoDB" },
	{ source: "Immerse", target: "Wikipedia" },
	{ source: "Immerse", target: "HappyCow" },
	{ source: "Immerse", target: "Lemmy" },
	{ source: "Give", target: "Wikipedia" },
	{ source: "Give", target: "Codeberg" },
	{ source: "Give", target: "Tangled" },
	{ source: "Give", target: "Radikale Venstre" },
	{ source: "Give", target: "HappyCow" },
	{ source: "Improve", target: "Codeberg" },
	{ source: "Improve", target: "Tangled" },
	{ source: "Improve", target: "NeoDB" },
	{ source: "Bluesky", target: "Atmosphere" },
	{ source: "Tangled", target: "Atmosphere" },
	{ source: "Rocksky", target: "Atmosphere" },
	{ source: "Bridgy", target: "Atmosphere" },
	{ source: "Atmosphere", target: "Bridgy" },
	{ source: "Fediverse", target: "Bridgy" },
	{ source: "Bridgy", target: "Fediverse" },
	{ source: "PixelFed", target: "Fediverse" },
	{ source: "Mastodon", target: "Fediverse" },
	{ source: "Lemmy", target: "Fediverse" },
	{ source: "NeoDB", target: "Fediverse" },
];

const graphData: GraphData = {
	nodes,
	links,
};

const Graph = () => {
	const goto = (url: string) => {
		window.location.href = url;
	};

	return (
		<ForceGraph3D
			graphData={graphData}
			nodeLabel={(node) => {
				return `<b style="white-space: pre; color: #ffffff; display: flex; text-align: center; font-size: 30px; text-shadow: 0 0 5px #000000, 2px 2px 18px #ff0072;">${node.desc}</b>`;
			}}
			backgroundColor="#222222"
			linkDirectionalParticles={2}
			linkDirectionalParticleWidth={1}
			onNodeClick={(node) => node.url && goto(node.url)}
			nodeThreeObject={(node) => {
				if (!node.color) {
					const imgTexture = new THREE.TextureLoader().load(
						`icons/${node.icon}`,
						(texture) => {
							texture.colorSpace = THREE.SRGBColorSpace;
						},
					);
					const material = new THREE.SpriteMaterial({ map: imgTexture });
					const sprite = new THREE.Sprite(material);
					const size = node.id === "Niclas Overby" ? 40 : 18;
					sprite.scale.set(size, size, 0);

					return sprite;
				} else {
					const group = new THREE.Group();
					const imgTexture = new THREE.TextureLoader().load(
						`icons/${node.icon}`,
						(texture) => {
							texture.colorSpace = THREE.SRGBColorSpace;
						},
					);
					const material = new THREE.SpriteMaterial({ map: imgTexture });
					const icon = new THREE.Sprite(material);
					icon.scale.set(20, 20, 0);
					group.add(
						icon,
						new THREE.Mesh(
							new THREE.SphereGeometry(15),
							new THREE.MeshLambertMaterial({
								color: node.color,
								transparent: true,
								opacity: node.opacity ?? 0.4,
							}),
						),
					);

					return group;
				}
			}}
		/>
	);
};

export default Graph;
