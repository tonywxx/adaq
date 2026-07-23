// ============================================
// 3. React Query 配置 (config/react-query.ts)
// ============================================

import { QueryClient, DefaultOptions } from '@tanstack/react-query'

const queryConfig: DefaultOptions = {
  queries: {
    // 数据被认为是新鲜的时间（5分钟）
    staleTime: 5 * 60 * 1000,
    // 缓存时间（10分钟）
    gcTime: 10 * 60 * 1000,
    // 失败重试次数
    retry: 3,
    // 重试延迟
    retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
    // 窗口聚焦时重新获取
    refetchOnWindowFocus: false,
    // 网络重连时重新获取
    refetchOnReconnect: true,
    // 挂载时重新获取
    refetchOnMount: true
  },
  mutations: {
    // mutation 失败重试
    retry: 1
  }
}

export const queryClient = new QueryClient({
  defaultOptions: queryConfig
})
