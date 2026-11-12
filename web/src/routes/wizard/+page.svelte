<script lang="ts">
	import { onMount } from 'svelte';
	import { toast } from 'svelte-sonner';
	import api, {
		type DirPreset,
		type WizardCandidate,
		type WizardResolveResponse
	} from '$lib/api';
	import { setBreadcrumb } from '$lib/stores/breadcrumb';

	let url = '';
	let resolving = false;
	let subscribing = false;
	let downloading = false;
	let resolved: WizardResolveResponse | null = null;
	let dirs: DirPreset[] = [];
	let dirId: number | null = null;
	let followNew = false;
	let forcePath = false;
	let results: import('$lib/api').WizardSubscribeItemResult[] = [];
	let selected = new Set<number>();

	async function loadDirs() {
		try {
			const res = await api.getDirs();
			dirs = res.data || [];
			if (dirId === null && dirs.length > 0) dirId = dirs[0].id;
		} catch (e: any) {
			toast.error('加载目录失败', { description: e?.message ?? String(e) });
		}
	}

	async function resolve() {
		if (!url.trim()) {
			toast.error('请先粘贴地址');
			return;
		}
		resolving = true;
		resolved = null;
		selected = new Set<number>();
		try {
			const res = await api.wizardResolve(url.trim());
			resolved = res.data || null;
			if (resolved?.candidates?.length) {
				// 默认全选
				selected = new Set(resolved.candidates.map((c) => c.sid));
			} else if (resolved?.sid) {
				selected = new Set([resolved.sid]);
			}
			if (resolved?.hint) toast.info(resolved.hint);
		} catch (e: any) {
			toast.error('解析失败', { description: e?.message ?? String(e) });
		} finally {
			resolving = false;
		}
	}

	function toggle(sid: number) {
		const next = new Set(selected);
		if (next.has(sid)) next.delete(sid);
		else next.add(sid);
		selected = next;
	}

	function selectAll(list: WizardCandidate[], on: boolean) {
		selected = on ? new Set(list.map((c) => c.sid)) : new Set<number>();
	}

	async function subscribe() {
		if (!resolved?.mid) {
			toast.error('缺少 UP ID，无法订阅');
			return;
		}
		if (dirId === null) {
			toast.error('请选择目录（没有目录请先去「目录」页创建）');
			return;
		}
		if (selected.size === 0) {
			toast.error('请至少勾选一个合集');
			return;
		}
		const items = (resolved.candidates || [])
			.filter((c) => selected.has(c.sid))
			.map((c) => ({ sid: c.sid, collection_type: c.collection_type, name: c.name }));
		if (items.length === 0 && resolved.sid) {
			items.push({
				sid: resolved.sid,
				collection_type: resolved.collection_type ?? 2,
				name: resolved.name ?? ''
			});
		}
		subscribing = true;
		try {
			const res = await api.wizardSubscribe({
				dir_id: dirId,
				mid: resolved.mid,
				upper_name: resolved.upper_name ?? undefined,
				items,
				follow_new: followNew,
				force_path: forcePath
			});
			const created = res.data?.created ?? [];
			const updated = res.data?.updated ?? [];
			const parts = [
				created.length ? `新增 ${created.length} 个` : '',
				updated.length ? `更新 ${updated.length} 个（已订阅过）` : ''
			].filter(Boolean);
			results = res.data?.items ?? [];
			toast.success(`${parts.join('，') || '未变更'}${res.data?.submission_created ? '，并开启追新' : ''}`, {
				description: '明细见下方列表'
			});
			selected = new Set<number>();
		} catch (e: any) {
			toast.error('订阅失败', { description: e?.message ?? String(e) });
		} finally {
			subscribing = false;
		}
	}

	async function downloadSingle() {
		if (!resolved?.bvid) {
			toast.error('缺少 BV 号');
			return;
		}
		if (dirId === null) {
			toast.error('请选择目录');
			return;
		}
		downloading = true;
		try {
			const res = await api.downloadVideo({
				bvid: resolved.bvid,
				dir_id: dirId,
				follow_new: followNew
			});
			const d = res.data;
			toast.success(`已开始下载：${d?.name ?? resolved.bvid}`, {
				description: `落盘目录 ${d?.path ?? '-'}${d?.follow_new ? '；已开启追新（订阅该 UP 投稿）' : ''}`
			});
		} catch (e: any) {
			toast.error('下载失败', { description: e?.message ?? String(e) });
		} finally {
			downloading = false;
		}
	}

	const kindText = (k?: string) =>
		k === 'upper'
			? 'UP 空间'
			: k === 'collection'
				? '合集'
				: k === 'series'
					? '系列'
					: k === 'video'
						? '单个视频'
						: k === 'favorite'
							? '收藏夹'
							: '未识别';

	onMount(() => {
		setBreadcrumb([{ label: '订阅向导' }]);
		loadDirs();
	});
</script>

<div class="mx-auto w-full max-w-5xl space-y-6 p-4">
	<header class="space-y-1">
		<h1 class="text-xl font-semibold">订阅向导</h1>
		<p class="text-muted-foreground text-sm">
			粘贴 B站 地址即可：UP 空间（自动列出全部合集）、合集、系列、单个视频。
			选好目录后一键订阅，下载结构为 <code class="text-xs">目录 / 作者名-作者ID / 合集名</code>。
		</p>
	</header>

	<section class="rounded-xl border p-4">
		<div class="flex flex-col gap-2 sm:flex-row">
			<input
				class="bg-background text-foreground w-full rounded-md border px-3 py-2 text-sm"
				placeholder="https://space.bilibili.com/3546913471203629 或 合集/视频地址"
				bind:value={url}
			/>
			<button
				class="shrink-0 rounded-md bg-blue-600 px-4 py-2 text-sm text-white disabled:opacity-60"
				onclick={resolve}
				disabled={resolving}
			>
				{resolving ? '解析中…' : '解析'}
			</button>
		</div>
	</section>

	{#if resolved}
		<section class="rounded-xl border p-4 text-sm">
			<div class="flex flex-wrap items-center gap-3">
				<span class="rounded bg-muted px-2 py-0.5 text-xs">{kindText(resolved.kind)}</span>
				{#if resolved.upper_name}
					<span>UP：{resolved.upper_name}</span>
				{/if}
				{#if resolved.mid}
					<span class="text-muted-foreground">mid：{resolved.mid}</span>
				{/if}
				{#if resolved.name}
					<span>{resolved.name}</span>
				{/if}
				{#if resolved.bvid}
					<span class="text-muted-foreground">{resolved.bvid}</span>
				{/if}
			</div>
			{#if resolved.hint}
				<p class="mt-2 text-amber-600">{resolved.hint}</p>
			{/if}
		</section>

		{#if resolved.candidates?.length}
			<section class="rounded-xl border p-4">
				<div class="mb-3 flex items-center justify-between text-sm">
					<h2 class="font-medium">选要订阅的合集（{resolved.candidates.length} 个）</h2>
					<div class="flex gap-3 text-xs">
						<button class="underline" onclick={() => selectAll(resolved!.candidates, true)}>全选</button>
						<button class="underline" onclick={() => selectAll(resolved!.candidates, false)}>全不选</button>
					</div>
				</div>
				<ul class="space-y-1">
					{#each resolved.candidates as c (c.sid)}
						<li class="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-muted/50">
							<input
								type="checkbox"
								checked={selected.has(c.sid)}
								onchange={() => toggle(c.sid)}
							/>
							<span class="flex-1 truncate">{c.name}</span>
							<span class="text-muted-foreground shrink-0 text-xs">{c.count} 集</span>
							<span class="text-muted-foreground shrink-0 text-xs">sid {c.sid}</span>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		<section class="rounded-xl border p-4">
			<div class="grid gap-4 text-sm md:grid-cols-2">
				<label class="block">
					<span class="text-muted-foreground">下载到目录</span>
					<select
						class="bg-background text-foreground mt-1 w-full rounded-md border px-2 py-2"
						bind:value={dirId}
					>
						{#if dirs.length === 0}
							<option class="bg-popover text-popover-foreground" value={null}>（还没有目录，请先去「目录」页创建）</option>
						{/if}
						{#each dirs as d (d.id)}
							<option class="bg-popover text-popover-foreground" value={d.id}>{d.alias} — {d.local_path}</option>
						{/each}
					</select>
				</label>
				<div class="mt-6 space-y-2">
					<label class="flex items-center gap-2">
						<input type="checkbox" bind:checked={followNew} />
						<span>追新：以后这个 UP 发新视频也自动下载</span>
					</label>
					<label class="flex items-center gap-2">
						<input type="checkbox" bind:checked={forcePath} />
						<span>已订阅过的合集：强制改成当前目录（默认保留原路径）</span>
					</label>
				</div>
			</div>
			<div class="mt-4">
				{#if resolved.kind === 'video'}
					<button
						class="rounded-md bg-emerald-600 px-4 py-2 text-sm text-white disabled:opacity-60"
						onclick={downloadSingle}
						disabled={downloading}
					>
						{downloading ? '提交中…' : '只下载这一个视频'}
					</button>
				{:else}
					<button
						class="rounded-md bg-emerald-600 px-4 py-2 text-sm text-white disabled:opacity-60"
						onclick={subscribe}
						disabled={subscribing}
					>
						{subscribing ? '订阅中…' : `订阅选中的 ${selected.size} 个`}
					</button>
				{/if}
			</div>

			{#if results.length > 0}
				<div class="mt-5 border-t pt-4 text-sm">
					<h3 class="mb-2 font-medium">本次结果</h3>
					<ul class="space-y-2">
						{#each results as r (r.sid)}
							<li class="rounded-md border p-2">
								<div class="flex items-center gap-2">
									<span
										class="rounded px-1.5 py-0.5 text-xs {r.action === 'created'
											? 'bg-emerald-600 text-white'
											: 'bg-amber-500 text-white'}"
									>
										{r.action === 'created' ? '新增' : '已存在'}
									</span>
									<span class="font-medium">{r.name}</span>
									<span class="text-muted-foreground text-xs">sid {r.sid}</span>
								</div>
								<div class="text-muted-foreground mt-1 text-xs">
									归属：{r.upper_name || '-'} ({r.mid})｜路径：{r.path}
								</div>
								<div class="text-muted-foreground text-xs">{r.note}</div>
							</li>
						{/each}
					</ul>
			</div>
			{/if}
		</section>
	{/if}
</div>
