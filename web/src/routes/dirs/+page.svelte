<script lang="ts">
	import { onMount } from 'svelte';
	import { toast } from 'svelte-sonner';
	import api, { type DirPreset, type DirPresetForm } from '$lib/api';
	import { setBreadcrumb } from '$lib/stores/breadcrumb';

	let dirs: DirPreset[] = [];
	let loading = false;
	let saving = false;
	let editingId: number | null = null;

	let form: DirPresetForm = emptyForm();

	function emptyForm(): DirPresetForm {
		return {
			alias: '',
			local_path: '',
			target115_path: '',
			default_season: 0,
			upper_display: 1,
			sort_order: 0
		};
	}

	async function loadDirs() {
		loading = true;
		try {
			const res = await api.getDirs();
			dirs = res.data || [];
		} catch (e: any) {
			toast.error('加载目录失败', { description: e?.message ?? String(e) });
		} finally {
			loading = false;
		}
	}

	function startAdd() {
		editingId = null;
		form = emptyForm();
		form.sort_order = dirs.length + 1;
	}

	function startEdit(d: DirPreset) {
		editingId = d.id;
		form = {
			alias: d.alias,
			local_path: d.local_path,
			target115_path: d.target115_path,
			default_season: d.default_season,
			upper_display: d.upper_display,
			sort_order: d.sort_order
		};
	}

	async function save() {
		if (!form.alias.trim() || !form.local_path.trim()) {
			toast.error('别名和本地路径必填');
			return;
		}
		saving = true;
		try {
			if (editingId === null) {
				await api.createDir(form);
				toast.success('目录已创建');
			} else {
				await api.updateDir(editingId, form);
				toast.success('目录已更新');
			}
			startAdd();
			await loadDirs();
		} catch (e: any) {
			toast.error('保存失败', { description: e?.message ?? String(e) });
		} finally {
			saving = false;
		}
	}

	async function remove(d: DirPreset) {
		if (!confirm(`删除目录「${d.alias}」？已订阅的路径不会自动改变。`)) return;
		try {
			await api.deleteDir(d.id);
			toast.success('已删除');
			await loadDirs();
		} catch (e: any) {
			toast.error('删除失败', { description: e?.message ?? String(e) });
		}
	}

	const upperDisplayText = (v: number) => (v === 0 ? 'UP 名' : v === 2 ? '仅 UP ID' : 'UP 名-ID');
	const localRoot = '/downloads/bili';

	onMount(() => {
		setBreadcrumb([{ label: '目录' }]);
		loadDirs();
	});
</script>

<div class="mx-auto w-full max-w-5xl space-y-6 p-4">
	<header class="space-y-1">
		<h1 class="text-xl font-semibold">目录</h1>
		<p class="text-muted-foreground text-sm">
			先在这里定义「目录」（数学 / 英语 / 语文 …），订阅时直接选目录即可。
			下载结构：<code class="text-xs">目录 / 作者名-作者ID / 合集或系列 / 视频名</code>。
			115 路径只作映射记录（上传由 MoviePilot 插件完成），可留空。
		</p>
	</header>

	<div class="grid gap-6 md:grid-cols-[1fr_1.2fr]">
		<section class="rounded-xl border p-4">
			<h2 class="mb-3 text-sm font-medium">{editingId === null ? '新增目录' : '编辑目录'}</h2>
			<div class="space-y-3 text-sm">
				<label class="block">
					<span class="text-muted-foreground">目录别名</span>
					<input
						class="bg-background text-foreground mt-1 w-full rounded-md border px-3 py-2"
						placeholder="英语"
						bind:value={form.alias}
					/>
				</label>
				<label class="block">
					<span class="text-muted-foreground">本地路径（容器内）</span>
					<input
						class="bg-background text-foreground mt-1 w-full rounded-md border px-3 py-2"
						placeholder="{localRoot}/英语"
						bind:value={form.local_path}
					/>
				</label>
				<label class="block">
					<span class="text-muted-foreground">115 路径（仅映射记录，可留空）</span>
					<input
						class="bg-background text-foreground mt-1 w-full rounded-md border px-3 py-2"
						placeholder="/媒体库/B站/英语"
						bind:value={form.target115_path}
					/>
				</label>
				<div class="grid grid-cols-3 gap-2">
					<label class="block">
						<span class="text-muted-foreground">默认季号</span>
						<input
							type="number"
							min="0"
							class="bg-background text-foreground mt-1 w-full rounded-md border px-3 py-2"
							bind:value={form.default_season}
						/>
					</label>
					<label class="block">
						<span class="text-muted-foreground">作者目录</span>
						<select
							class="bg-background text-foreground mt-1 w-full rounded-md border px-2 py-2"
							bind:value={form.upper_display}
						>
							<option class="bg-popover text-popover-foreground" value={1}>UP 名-ID</option>
							<option class="bg-popover text-popover-foreground" value={0}>仅 UP 名</option>
							<option class="bg-popover text-popover-foreground" value={2}>仅 UP ID</option>
						</select>
					</label>
					<label class="block">
						<span class="text-muted-foreground">排序</span>
						<input
							type="number"
							class="bg-background text-foreground mt-1 w-full rounded-md border px-3 py-2"
							bind:value={form.sort_order}
						/>
					</label>
				</div>
				<div class="flex gap-2 pt-1">
					<button
						class="rounded-md bg-blue-600 px-3 py-2 text-white disabled:opacity-60"
						onclick={save}
						disabled={saving}
					>
						{saving ? '保存中…' : '保存'}
					</button>
					{#if editingId !== null}
						<button class="rounded-md border px-3 py-2" onclick={startAdd}>取消编辑</button>
					{/if}
				</div>
			</div>
		</section>

		<section class="rounded-xl border p-4">
			<div class="mb-3 flex items-center justify-between">
				<h2 class="text-sm font-medium">已有目录（{dirs.length}）</h2>
				<button class="text-muted-foreground text-xs underline" onclick={loadDirs}>刷新</button>
			</div>
			{#if loading}
				<p class="text-muted-foreground text-sm">加载中…</p>
			{:else if dirs.length === 0}
				<p class="text-muted-foreground text-sm">还没有目录，先在左边创建一个。</p>
			{:else}
				<ul class="space-y-2">
					{#each dirs as d (d.id)}
						<li class="rounded-lg border p-3 text-sm">
							<div class="flex items-start justify-between gap-3">
								<div class="min-w-0">
									<div class="font-medium">{d.alias}</div>
									<div class="text-muted-foreground truncate text-xs">
										本地：{d.local_path}
									</div>
									{#if d.target115_path}
										<div class="text-muted-foreground truncate text-xs">
											115：{d.target115_path}
										</div>
									{/if}
									<div class="text-muted-foreground text-xs">
										作者目录：{upperDisplayText(d.upper_display)}
										{#if d.default_season > 0}· 默认季 {d.default_season}{/if}
										· 排序 {d.sort_order}
									</div>
								</div>
								<div class="flex shrink-0 gap-2 text-xs">
									<button class="rounded border px-2 py-1" onclick={() => startEdit(d)}>编辑</button
									>
									<button class="rounded border px-2 py-1 text-rose-600" onclick={() => remove(d)}
										>删除</button
									>
								</div>
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	</div>
</div>
