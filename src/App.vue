<script setup lang="ts">
import { ref } from 'vue'
import FlashView from './views/FlashView.vue'
import LogView from './views/LogView.vue'
import FactoryView from './views/FactoryView.vue'
import SettingsView from './views/SettingsView.vue'

type ViewId = 'flash' | 'log' | 'factory' | 'settings'

const activeView = ref<ViewId>('flash')

const navItems: { id: ViewId; label: string; icon: string }[] = [
  { id: 'flash',   label: 'Download / Flash', icon: '⚡' },
  { id: 'log',     label: 'Log Viewer',        icon: '📋' },
  { id: 'factory', label: 'Factory Mode',      icon: '🏭' },
  { id: 'settings',label: 'Settings',          icon: '⚙️' },
]

// Simulated status info – will be driven by Tauri serial events later
const connectedPort = ref<string | null>(null)
const deviceName = ref<string | null>(null)
</script>

<template>
  <div class="flex flex-col h-screen bg-gray-950 text-gray-100 font-mono select-none overflow-hidden">

    <!-- ── Main layout (sidebar + content) ───────────────────────────── -->
    <div class="flex flex-1 overflow-hidden">

      <!-- Sidebar -->
      <aside class="w-52 shrink-0 flex flex-col bg-gray-900 border-r border-gray-800">
        <!-- Logo / title -->
        <div class="px-4 py-5 border-b border-gray-800">
          <span class="text-lg font-bold tracking-wide text-cyan-400">LuaTools</span>
          <span class="ml-1 text-xs text-gray-500">v4</span>
        </div>

        <!-- Navigation -->
        <nav class="flex-1 py-3 space-y-1 px-2">
          <button
            v-for="item in navItems"
            :key="item.id"
            @click="activeView = item.id"
            :class="[
              'w-full flex items-center gap-3 px-3 py-2.5 rounded-md text-sm transition-colors duration-150',
              activeView === item.id
                ? 'bg-cyan-600 text-white font-semibold'
                : 'text-gray-400 hover:bg-gray-800 hover:text-gray-100',
            ]"
          >
            <span class="text-base leading-none">{{ item.icon }}</span>
            <span>{{ item.label }}</span>
          </button>
        </nav>
      </aside>

      <!-- Main content area -->
      <main class="flex-1 overflow-auto bg-gray-950 p-6">
        <FlashView   v-if="activeView === 'flash'" />
        <LogView     v-else-if="activeView === 'log'" />
        <FactoryView v-else-if="activeView === 'factory'" />
        <SettingsView v-else-if="activeView === 'settings'" />
      </main>
    </div>

    <!-- ── Bottom status bar ─────────────────────────────────────────── -->
    <footer class="shrink-0 flex items-center gap-4 px-4 py-1.5 bg-gray-900 border-t border-gray-800 text-xs text-gray-400">
      <span
        :class="connectedPort ? 'text-green-400' : 'text-gray-600'"
        class="flex items-center gap-1.5"
      >
        <span
          :class="connectedPort ? 'bg-green-400' : 'bg-gray-600'"
          class="inline-block w-2 h-2 rounded-full"
        />
        {{ connectedPort ? connectedPort : 'No device connected' }}
      </span>
      <span v-if="deviceName" class="text-gray-500">{{ deviceName }}</span>
      <span class="ml-auto text-gray-700">LuatOS Flash Tool</span>
    </footer>

  </div>
</template>
