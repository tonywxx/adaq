"use client";

import type * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
	SidebarGroup,
	SidebarGroupContent,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
} from "@/components/ui/sidebar";

export function NavSecondary({
	items,
	...props
}: {
	items: {
		title: string;
		url: string;
		icon: React.ReactNode;
		active?: boolean;
	}[];
} & React.ComponentPropsWithoutRef<typeof SidebarGroup>) {
	const navigate = useNavigate();
	function open(item: { url: string }) {
		if (item.url.startsWith("/")) {
			void navigate({ to: item.url });
			return;
		}
		void openUrl(item.url);
	}
	return (
		<SidebarGroup {...props}>
			<SidebarGroupContent>
				<SidebarMenu>
					{items.map((item) => (
						<SidebarMenuItem key={item.title}>
							<SidebarMenuButton onClick={() => open(item)} isActive={item.active}>
								{item.icon}
								<span>{item.title}</span>
							</SidebarMenuButton>
						</SidebarMenuItem>
					))}
				</SidebarMenu>
			</SidebarGroupContent>
		</SidebarGroup>
	);
}
