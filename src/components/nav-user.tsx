import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	useSidebar,
} from "@/components/ui/sidebar";
import { supabase } from "@/lib/supabase";
import type { User } from "@supabase/supabase-js";
import {
	CircleUserRoundIcon,
	EllipsisVerticalIcon,
	LogOutIcon,
	Settings2Icon,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function NavUser() {
	const { isMobile } = useSidebar();
	const navigate = useNavigate();
	const { t } = useTranslation();
	const [authUser, setAuthUser] = useState<User | null>(null);
	const user = userInfo(authUser, t("nav.signedIn"), t("nav.account"));

	useEffect(() => {
		if (!supabase) return;

		supabase.auth.getUser().then(({ data }) => setAuthUser(data.user));
		const {
			data: { subscription },
		} = supabase.auth.onAuthStateChange((_event, session) => {
			setAuthUser(session?.user ?? null);
		});

		return () => subscription.unsubscribe();
	}, []);

	return (
		<SidebarMenu>
			<SidebarMenuItem>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<SidebarMenuButton
							size="lg"
							className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
						>
							<Avatar className="h-8 w-8 rounded-lg grayscale">
								<AvatarImage src={user.avatar} alt={user.name} />
								<AvatarFallback className="rounded-lg">{user.initials}</AvatarFallback>
							</Avatar>
							<div className="grid flex-1 text-left text-sm leading-tight">
								<span className="truncate font-medium">{user.name}</span>
								<span className="truncate text-xs text-muted-foreground">
									{/* {user.email} */}
								</span>
							</div>
							<EllipsisVerticalIcon className="ml-auto size-4" />
						</SidebarMenuButton>
					</DropdownMenuTrigger>
					<DropdownMenuContent
						className="w-(--radix-dropdown-menu-trigger-width) min-w-56 rounded-lg"
						side={isMobile ? "bottom" : "right"}
						align="end"
						sideOffset={4}
					>
						<DropdownMenuGroup>
							<DropdownMenuLabel className="p-0 font-normal">
								<div className="flex items-center gap-2 px-1 py-1.5 text-left text-sm">
									<Avatar className="h-8 w-8 rounded-lg">
										<AvatarImage src={user.avatar} alt={user.name} />
										<AvatarFallback className="rounded-lg">
											{user.initials}
										</AvatarFallback>
									</Avatar>
									<div className="grid flex-1 text-left text-sm leading-tight">
										<span className="truncate font-medium">{user.name}</span>
										<span className="truncate text-xs text-muted-foreground">
											{user.email}
										</span>
									</div>
								</div>
							</DropdownMenuLabel>
							<DropdownMenuItem
								onClick={() =>
									void navigate({
										to: "/settings/$section",
										params: { section: "account" },
									})
								}
							>
								<CircleUserRoundIcon />
								{t("nav.account")}
							</DropdownMenuItem>
							<DropdownMenuItem
								onClick={() =>
									void navigate({
										to: "/settings/$section",
										params: { section: "general" },
									})
								}
							>
								<Settings2Icon />
								{t("nav.settings")}
							</DropdownMenuItem>
						</DropdownMenuGroup>
						<DropdownMenuSeparator />
						<DropdownMenuItem onClick={() => supabase?.auth.signOut()}>
							<LogOutIcon />
							{t("nav.logOut")}
						</DropdownMenuItem>
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}

function userInfo(
	user: User | null,
	signedInLabel: string,
	accountLabel: string,
) {
	const email = user?.email ?? signedInLabel;
	const name =
		user?.user_metadata.full_name ??
		user?.user_metadata.name ??
		email.split("@")[0] ??
		accountLabel;
	const avatar =
		user?.user_metadata.avatar_url ?? user?.user_metadata.picture ?? undefined;

	return {
		name,
		email,
		avatar,
		initials: initials(name || email),
	};
}

function initials(value: string) {
	return value
		.split(/\s+|@/)
		.filter(Boolean)
		.slice(0, 2)
		.map((part) => part[0]?.toUpperCase())
		.join("");
}
